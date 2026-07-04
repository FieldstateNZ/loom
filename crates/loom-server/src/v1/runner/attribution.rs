//! Usage attribution and recording for a completed turn.
//!
//! [`UsageAttribution`] is the identity a turn's usage is recorded against;
//! [`record_turn_usage`] prices and records it (best effort). Shared by the
//! non-streaming path in [`super`] and the streamed-turn settlement in
//! [`super::stream`].

use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

use loom_core::Usage;
use loom_store::{NewUsageEvent, Pricer, PricingStore};

use crate::state::AppState;

/// The identity a turn's usage is attributed to.
///
/// `conversation_id` is `Some` only for stateful turns; a stateless turn
/// records usage against the tenant and key with no conversation.
#[derive(Clone)]
pub(super) struct UsageAttribution {
    pub(super) tenant_id: Uuid,
    pub(super) virtual_key_id: Option<Uuid>,
    pub(super) conversation_id: Option<Uuid>,
    pub(super) provider: String,
    pub(super) model: String,
}

/// Records a priced usage event for a completed turn (best effort).
///
/// The cost is computed at write time from the effective price for
/// `(provider, model)` at the current instant; if no price is configured or the
/// lookup fails, the event is still recorded with `cost = None` so the raw usage
/// is never lost and cost can be recomputed later. The write itself goes through
/// the state's [`UsageRecorder`](crate::usage::UsageRecorder), which parks the
/// event in the outbox on failure — a usage-write fault never fails the turn.
pub(super) async fn record_turn_usage(
    state: &AppState,
    attribution: &UsageAttribution,
    usage: Usage,
) -> Option<Decimal> {
    let cost = match state
        .store()
        .get_effective_price(&attribution.provider, &attribution.model, Utc::now())
        .await
    {
        Ok(Some(price)) => Some(Pricer::cost(&usage, &price)),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(error = %err, "price lookup failed; recording usage without cost");
            None
        }
    };
    // Debit the key's per-minute token bucket now that the turn's usage is known
    // (a no-op when the key has no token limit).
    if let Some(key_id) = attribution.virtual_key_id {
        let tokens = usage
            .input_tokens
            .unwrap_or(0)
            .saturating_add(usage.output_tokens.unwrap_or(0))
            .saturating_add(usage.cache_read_tokens.unwrap_or(0))
            .saturating_add(usage.cache_write_tokens.unwrap_or(0));
        state.rate_limiter().record_tokens(key_id, tokens);
    }
    // Emit token/cost metrics by tenant + model (no-ops without a meter).
    state.metrics().record_tokens(
        attribution.tenant_id,
        &attribution.model,
        usage.input_tokens.unwrap_or(0),
        usage.output_tokens.unwrap_or(0),
    );
    if let Some(cost) = cost {
        state
            .metrics()
            .record_cost(attribution.tenant_id, &attribution.model, cost);
    }
    let event = NewUsageEvent {
        tenant_id: attribution.tenant_id,
        virtual_key_id: attribution.virtual_key_id,
        conversation_id: attribution.conversation_id,
        provider: attribution.provider.clone(),
        model: attribution.model.clone(),
        usage,
        cost,
        is_batch: false,
    };
    state.usage_recorder().record(state.store(), event).await;
    cost
}
