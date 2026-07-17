//! Persistence for the pending tool-call set a session is parked on.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::model::NewPendingToolCall;
use loom_core::PendingToolCall;

/// The outcome of a `sendToolResult` correlation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// The matching pending call was found and marked resolved.
    Resolved,
    /// No call with that `tool_use_id` exists for the session (an unknown id, or
    /// the session is missing or belongs to another tenant).
    NotFound,
    /// A call with that `tool_use_id` exists but was already resolved — a
    /// duplicate result.
    AlreadyResolved,
}

/// Read/write access to a session's pending tool-call set, scoped to a tenant.
///
/// The substrate for `getPendingToolCalls`, `sendToolResult` correlation, and
/// `drain`: a call is pending from [`record_pending`](PendingToolCallStore::record_pending)
/// until its result is posted via [`resolve`](PendingToolCallStore::resolve).
#[async_trait]
pub trait PendingToolCallStore {
    /// Records a blocking tool call on a session. Returns the recorded call, or
    /// `None` if the session does not exist or belongs to another tenant.
    async fn record_pending(
        &self,
        tenant_id: Uuid,
        session_id: Uuid,
        call: NewPendingToolCall,
    ) -> Result<Option<PendingToolCall>>;

    /// Enumerates a session's still-blocking (unresolved) tool calls,
    /// oldest-first, scoped to a tenant. This is `getPendingToolCalls`.
    async fn list_pending(&self, tenant_id: Uuid, session_id: Uuid)
        -> Result<Vec<PendingToolCall>>;

    /// Correlates a result to a pending call by `tool_use_id` and marks it
    /// resolved. This is `sendToolResult`'s correlation step: it rejects an
    /// unknown or already-resolved id (see [`ResolveOutcome`]).
    async fn resolve(
        &self,
        tenant_id: Uuid,
        session_id: Uuid,
        tool_use_id: &str,
        result: serde_json::Value,
        is_error: bool,
    ) -> Result<ResolveOutcome>;
}
