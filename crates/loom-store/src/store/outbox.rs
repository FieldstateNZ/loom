//! Persistence for the usage outbox — the failure-mode safety net.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::model::{NewUsageEvent, OutboxEntry};

/// Persistence for the usage outbox — the failure-mode safety net.
///
/// A usage-write failure must never fail the user's turn. When the primary
/// [`UsageStore::record_event`](crate::UsageStore::record_event) write fails,
/// the event is parked here and a drain pass
/// ([`crate::drain_usage_outbox`]) replays it later.
// `async_trait` (<= 0.1.89) stamps a bare `#[must_use]` on each async trait
// method while the boxed future it returns is already `#[must_use]`, which
// trips `clippy::double_must_use` on newer toolchains. The attribute is
// macro-generated, so this scoped allow is the narrowest place to quiet it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait OutboxStore {
    /// Parks a usage event in the outbox (status `pending`), returning its id.
    async fn enqueue_outbox(&self, event: &NewUsageEvent) -> Result<Uuid>;

    /// Lists pending outbox entries oldest-first, capped by `limit`.
    async fn list_pending_outbox(&self, limit: i64) -> Result<Vec<OutboxEntry>>;

    /// Marks an outbox entry processed (drained successfully).
    async fn mark_outbox_processed(&self, id: Uuid) -> Result<()>;

    /// Records a failed drain attempt: bumps the attempt count and stores the
    /// error, leaving the entry pending for a later retry.
    async fn mark_outbox_failed(&self, id: Uuid, error: &str) -> Result<()>;
}
