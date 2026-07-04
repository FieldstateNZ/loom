//! Native Anthropic server-tool and MCP connector entries: the
//! provider-executed [`ServerTool`]s and external [`McpServerRef`]s that ride
//! alongside client tools in a request.

use loom_core::{McpServerRef, ServerTool};
use serde_json::{json, Map, Value};

/// The native `type` for Anthropic's web search server tool.
const WEB_SEARCH_TOOL_TYPE: &str = "web_search_20250305";
/// The native `type` for Anthropic's code execution server tool.
const CODE_EXECUTION_TOOL_TYPE: &str = "code_execution_20250522";

/// Maps a [`ServerTool`] to its native Anthropic versioned tool entry.
///
/// [`ServerTool::WebSearch`] and [`ServerTool::CodeExecution`] map to their
/// native `{ "type": "<versioned>", "name": … }` shapes; [`ServerTool::Raw`]
/// forwards its wrapped native definition verbatim so a caller can drive a
/// server tool Loom does not model yet.
pub(super) fn server_tool_to_native(tool: &ServerTool) -> Value {
    match tool {
        ServerTool::WebSearch {
            max_uses,
            allowed_domains,
            blocked_domains,
        } => {
            let mut obj = Map::new();
            obj.insert("type".into(), json!(WEB_SEARCH_TOOL_TYPE));
            obj.insert("name".into(), json!("web_search"));
            if let Some(max_uses) = max_uses {
                obj.insert("max_uses".into(), json!(max_uses));
            }
            if let Some(allowed_domains) = allowed_domains {
                obj.insert("allowed_domains".into(), json!(allowed_domains));
            }
            if let Some(blocked_domains) = blocked_domains {
                obj.insert("blocked_domains".into(), json!(blocked_domains));
            }
            Value::Object(obj)
        }
        ServerTool::CodeExecution {} => {
            json!({ "type": CODE_EXECUTION_TOOL_TYPE, "name": "code_execution" })
        }
        // The escape hatch carries the native tool definition verbatim.
        ServerTool::Raw(definition) => definition.clone(),
        // `ServerTool` is `#[non_exhaustive]`; a future variant Loom does not
        // yet map is emitted as an empty object rather than panicking.
        _ => json!({}),
    }
}

/// Maps an [`McpServerRef`] to Anthropic's native `mcp_servers` entry.
///
/// Anthropic's connector expects `{ "type": "url", "name", "url",
/// "authorization_token"?, "tool_configuration"? }`. The authorization token is
/// emitted only when present — for a named reference it has been injected
/// upstream after decryption; for an inline reference the caller supplied it.
/// The token is a bearer secret and appears **only** in the outbound request
/// body, never in a response or in persisted history.
pub(super) fn mcp_server_to_native(server: &McpServerRef) -> Value {
    let mut obj = Map::new();
    obj.insert("type".into(), json!("url"));
    obj.insert("name".into(), json!(server.name));
    if let Some(url) = &server.url {
        obj.insert("url".into(), json!(url));
    }
    if let Some(authorization) = &server.authorization {
        obj.insert("authorization_token".into(), json!(authorization));
    }
    if let Some(tool_configuration) = &server.tool_configuration {
        obj.insert("tool_configuration".into(), tool_configuration.clone());
    }
    Value::Object(obj)
}
