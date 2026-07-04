//! Best-effort usage recording with an outbox fallback.
//!
//! A usage-write failure must never fail the user's turn. The [`UsageRecorder`]
//! abstraction records a priced usage event and, on failure, parks it in the
//! store's usage outbox for a later drain — always returning to the caller
//! without surfacing an error. Tests substitute their own recorder (mirroring
//! the [`ProviderFactory`](crate::provider::ProviderFactory) injection pattern)
//! to exercise the failure path deterministically.

use async_trait::async_trait;

use loom_store::{NewUsageEvent, OutboxStore, PgStore, UsageStore};

/// Records a usage event for a completed turn. Implementations are
/// **best-effort**: `record` returns `()` and never propagates an error, so a
/// persistence failure can never fail the turn that produced the usage.
#[async_trait]
pub trait UsageRecorder: Send + Sync {
    /// Records `event`, swallowing and logging any failure.
    async fn record(&self, store: &PgStore, event: NewUsageEvent);
}

/// The default recorder: write to `usage_events`; on failure, park the event in
/// the usage outbox so a drain pass can settle it later.
#[derive(Clone, Copy, Debug, Default)]
pub struct OutboxUsageRecorder;

#[async_trait]
impl UsageRecorder for OutboxUsageRecorder {
    async fn record(&self, store: &PgStore, event: NewUsageEvent) {
        if let Err(err) = store.record_event(event.clone()).await {
            tracing::warn!(error = %err, "usage event write failed; parking in outbox");
            if let Err(outbox_err) = store.enqueue_outbox(&event).await {
                // Both the primary write and the outbox park failed: the usage
                // is lost, but the turn still succeeds. Log loudly.
                tracing::error!(error = %outbox_err, "usage outbox enqueue failed; usage lost");
            }
        }
    }
}
