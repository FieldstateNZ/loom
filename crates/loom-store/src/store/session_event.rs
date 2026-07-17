//! Persistence for the normalised session event log.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use loom_core::{Event, EventKind};

/// Append/read access to a session's normalised [`Event`] log, scoped to a
/// tenant.
///
/// The log is the durable backing for `stream` (SSE) and cursor-paginated
/// `listSessionEvents`, and doubles as an audit source. Each appended event gets
/// a per-session monotonic, order-comparable [`Event::id`].
#[async_trait]
pub trait SessionEventStore {
    /// Appends an event to a session's log and returns the stored [`Event`]
    /// (with its assigned id and timestamp). Returns `None` if the session does
    /// not exist or belongs to another tenant.
    async fn append_event(
        &self,
        tenant_id: Uuid,
        session_id: Uuid,
        kind: &EventKind,
    ) -> Result<Option<Event>>;

    /// Lists a session's events in order, scoped to a tenant.
    ///
    /// `after` is an exclusive cursor ([`Event::id`]); pass `None` to start from
    /// the beginning. `limit` caps the page size. Returns an empty vector if the
    /// session does not exist or belongs to another tenant.
    async fn list_session_events(
        &self,
        tenant_id: Uuid,
        session_id: Uuid,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Event>>;
}
