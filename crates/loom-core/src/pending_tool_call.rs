//! A blocking tool call a session is parked on, awaiting its result.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Whether a tool call targets a custom (agent-defined) tool or a
/// provider-hosted builtin.
///
/// The distinction is load-bearing for `drain` authorization: a builtin call may
/// be authorised even when its origin is unattributed, whereas a custom or MCP
/// call must match a pinned grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// A custom tool defined on the agent (includes MCP tools).
    Custom,
    /// A provider-hosted builtin tool.
    Builtin,
}

/// A blocking tool call a session is parked on, awaiting a result.
///
/// A session may block on one or more tool calls; each is enumerated by
/// `getPendingToolCalls` and cleared by `sendToolResult` (or `drain`). The
/// [`tool_use_id`](PendingToolCall::tool_use_id) is the correlation key a result
/// is matched against — unique within its session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PendingToolCall {
    /// The call's unique identifier.
    pub id: Uuid,

    /// The session this call blocks.
    pub session_id: Uuid,

    /// The provider's correlation id — a result is matched to this call by it.
    pub tool_use_id: String,

    /// Whether this is a custom or a builtin tool.
    pub kind: ToolKind,

    /// The tool's name.
    pub name: String,

    /// The input arguments the tool was invoked with.
    pub input: serde_json::Value,

    /// The MCP server URL this call originated from, when the adapter can vouch
    /// for it. Absent (rather than `null`) when unknown — an unattributed call
    /// is not fail-closed only under `drain`'s builtin carve-out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_server_url: Option<String>,

    /// When the call was recorded.
    pub created_at: DateTime<Utc>,
}
