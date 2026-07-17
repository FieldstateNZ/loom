//! Input types for pending-tool-call writes.

use loom_core::ToolKind;

/// Input to record a new blocking tool call on a session.
#[derive(Clone, Debug)]
pub struct NewPendingToolCall {
    /// The provider's correlation id.
    pub tool_use_id: String,
    /// Whether the tool is custom or builtin.
    pub kind: ToolKind,
    /// The tool's name.
    pub name: String,
    /// The input arguments the tool was invoked with.
    pub input: serde_json::Value,
    /// The MCP server origin, when the adapter can vouch for it.
    pub mcp_server_url: Option<String>,
}
