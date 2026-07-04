//! Budget enforcement: resolve the effective budget for a request, compute
//! current-window spend (through a short-TTL cache), and decide whether to
//! allow, warn, or block.
//!
//! A key-level budget overrides its tenant's default. Current spend is derived
//! from the `usage_events` rollup (the #9 store): a key-scoped budget meters
//! that key's spend, a tenant-scoped budget meters the whole tenant's spend.
//!
//! # The spend cache
//!
//! Enforcement runs before every turn, so a naive implementation would issue a
//! `SUM(cost)` query per request. Instead spend is memoised in an in-process
//! [`BudgetCache`] keyed by `(tenant, scope, window)` with a short TTL, so a
//! burst of turns shares one query. The cache is per-replica (like the rate
//! limiter); across replicas each holds its own view. A freshly recorded turn's
//! cost may not be reflected until the TTL lapses — acceptable for a spend
//! guard, and the window bound keeps the drift small.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

use loom_store::{Budget, BudgetAction, BudgetStore, BudgetWindow};

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::state::AppState;

/// How long a memoised spend figure stays fresh.
const CACHE_TTL: Duration = Duration::from_secs(5);

/// The cache key: a tenant, an optional key scope, and the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SpendKey {
    tenant_id: Uuid,
    key_id: Option<Uuid>,
    window: BudgetWindow,
}

/// A memoised spend figure and when it was computed.
#[derive(Clone, Copy, Debug)]
struct Cached {
    spent: Decimal,
    at: Instant,
}

/// A process-wide, short-TTL cache of current-window spend.
///
/// Cheap to share behind an [`Arc`](std::sync::Arc).
#[derive(Debug, Default)]
pub struct BudgetCache {
    entries: Mutex<HashMap<SpendKey, Cached>>,
}

impl BudgetCache {
    /// A fresh, empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a fresh cached spend for `key`, or `None` on a miss/expiry.
    fn get(&self, key: &SpendKey, now: Instant) -> Option<Decimal> {
        let entries = self.entries.lock().expect("budget cache mutex poisoned");
        entries
            .get(key)
            .filter(|c| now.saturating_duration_since(c.at) < CACHE_TTL)
            .map(|c| c.spent)
    }

    /// Stores a freshly-computed spend for `key`.
    fn put(&self, key: SpendKey, spent: Decimal, now: Instant) {
        let mut entries = self.entries.lock().expect("budget cache mutex poisoned");
        entries.insert(key, Cached { spent, at: now });
    }
}

/// A soft-limit warning produced when a `warn`-action budget is over limit.
///
/// The request is allowed; the caller surfaces [`header_value`](Self::header_value)
/// as the `x-loom-budget-warning` response header.
#[derive(Clone, Debug)]
pub struct BudgetWarning {
    message: String,
}

impl BudgetWarning {
    /// The `x-loom-budget-warning` header value.
    #[must_use]
    pub fn header_value(&self) -> &str {
        &self.message
    }
}

/// Evaluates the effective budget for a request, before the provider call.
///
/// Resolves the key-level budget (from the [`TenantContext`]) or, failing that,
/// the tenant default; computes current-window spend through the cache; and:
///
/// - returns `Err` with a `402 budget_exceeded` when an over-limit budget's
///   action is `block`;
/// - returns `Ok(Some(warning))` when an over-limit budget's action is `warn`
///   (the request proceeds, flagged);
/// - returns `Ok(None)` when under limit or no budget applies.
///
/// # Errors
///
/// Propagates a store error (as a `500`) if the tenant-budget lookup or the
/// spend query fails, and a `402` [`ApiError::budget_exceeded`] on a hard block.
pub async fn enforce(
    state: &AppState,
    ctx: &TenantContext,
) -> Result<Option<BudgetWarning>, ApiError> {
    // Resolve the effective budget: a key budget overrides the tenant default.
    // A key budget meters the key; the tenant default meters the whole tenant.
    let (scope, scope_key_id, budget) = if let Some(budget) = ctx.budget.clone() {
        ("key", Some(ctx.key_id), budget)
    } else {
        match state
            .store()
            .get_tenant_budget(ctx.tenant_id)
            .await
            .map_err(ApiError::from_store)?
        {
            Some(budget) => ("tenant", None, budget),
            None => return Ok(None),
        }
    };

    let spent = current_spend(state, ctx.tenant_id, scope_key_id, budget.window).await?;
    if spent < budget.limit_amount {
        return Ok(None);
    }

    let window = budget.window.as_str();
    match budget.action {
        BudgetAction::Block => Err(ApiError::budget_exceeded(
            scope,
            budget.limit_amount,
            spent,
            window,
        )),
        BudgetAction::Warn => {
            tracing::warn!(
                scope,
                tenant_id = %ctx.tenant_id,
                key_id = %ctx.key_id,
                limit = %budget.limit_amount,
                spent = %spent,
                window,
                "budget soft limit exceeded (warn action); allowing request"
            );
            Ok(Some(BudgetWarning {
                message: format!(
                    "{scope} budget of {limit} exceeded (spent {spent} this {window})",
                    limit = budget.limit_amount,
                ),
            }))
        }
    }
}

/// Current-window spend for a scope, memoised in the [`BudgetCache`].
async fn current_spend(
    state: &AppState,
    tenant_id: Uuid,
    scope_key_id: Option<Uuid>,
    window: BudgetWindow,
) -> Result<Decimal, ApiError> {
    let now = Instant::now();
    let cache_key = SpendKey {
        tenant_id,
        key_id: scope_key_id,
        window,
    };
    if let Some(spent) = state.budget_cache().get(&cache_key, now) {
        return Ok(spent);
    }
    let since = window.start(Utc::now());
    let spent = state
        .store()
        .budget_spend(tenant_id, scope_key_id, since)
        .await
        .map_err(ApiError::from_store)?;
    state.budget_cache().put(cache_key, spent, now);
    Ok(spent)
}

/// A [`Budget`] for the admin API, validated from string window/action inputs.
///
/// Kept here so both the enforcement path and the admin setter share one notion
/// of a valid budget.
///
/// # Errors
///
/// Returns a `400` [`ApiError`] if `window` or `action` is not a known value.
pub fn parse_budget(limit_amount: Decimal, window: &str, action: &str) -> Result<Budget, ApiError> {
    let window = BudgetWindow::parse(window).ok_or_else(|| {
        ApiError::bad_request(format!(
            "unknown budget window {window:?}; expected daily, weekly, monthly, or total"
        ))
    })?;
    let action = BudgetAction::parse(action).ok_or_else(|| {
        ApiError::bad_request(format!(
            "unknown budget action {action:?}; expected block or warn"
        ))
    })?;
    Ok(Budget {
        limit_amount,
        window,
        action,
    })
}
