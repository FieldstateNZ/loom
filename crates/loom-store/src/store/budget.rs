//! Persistence for budgets and rate limits, and the current-window spend
//! query that budget enforcement reads.

use async_trait::async_trait;
use uuid::Uuid;

use chrono::{DateTime, Utc};

use rust_decimal::Decimal;

use crate::error::Result;
use crate::model::{Budget, RateLimit};

/// Persistence for budgets and rate limits, and the current-window spend query
/// that budget enforcement reads.
///
/// Budgets attach at the tenant and the key level; a key-level budget overrides
/// the tenant default. Rate limits attach per key. Current spend is derived from
/// the `usage_events` rollup (the #9 store) at enforcement time — never
/// denormalised here.
#[async_trait]
pub trait BudgetStore {
    /// Fetches a tenant's default budget, or `None` if it has none.
    async fn get_tenant_budget(&self, tenant_id: Uuid) -> Result<Option<Budget>>;

    /// Sets (or, with `None`, clears) a tenant's default budget. Returns `true`
    /// if the tenant exists and was updated.
    async fn set_tenant_budget(&self, tenant_id: Uuid, budget: Option<Budget>) -> Result<bool>;

    /// Sets (or, with `None`, clears) a key's budget override. Returns `true`
    /// if the key exists and was updated.
    async fn set_key_budget(&self, key_id: Uuid, budget: Option<Budget>) -> Result<bool>;

    /// Sets (or, with `None`, clears) a key's rate limit. Returns `true` if the
    /// key exists and was updated.
    async fn set_key_rate_limit(&self, key_id: Uuid, rate_limit: Option<RateLimit>)
        -> Result<bool>;

    /// Sums the recorded cost of usage in the current budget window.
    ///
    /// Scoped to `tenant_id`; if `key_id` is `Some`, further scoped to that
    /// key. `since` is the inclusive lower bound on event time, or `None` for
    /// an open window (all recorded usage). Events with no computed cost
    /// contribute zero.
    async fn budget_spend(
        &self,
        tenant_id: Uuid,
        key_id: Option<Uuid>,
        since: Option<DateTime<Utc>>,
    ) -> Result<Decimal>;
}
