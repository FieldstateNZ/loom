//! Provider-executed (server-side) tool configuration.

use serde::{Deserialize, Serialize};

/// A **provider-executed** (server-side) tool Loom asks the provider to run on
/// the model's behalf.
///
/// This models the *configuration* of a server tool — what the caller offers —
/// as opposed to the [`ServerToolUse`](crate::ContentPart::ServerToolUse) /
/// [`ServerToolResult`](crate::ContentPart::ServerToolResult) content parts that
/// carry a server tool's *invocation* in a response. Provider translators map
/// each variant to the provider's native versioned tool entry.
///
/// # Extensibility
///
/// The enum is `#[non_exhaustive]`: new server tools will be added as providers
/// ship them. The [`Raw`](ServerTool::Raw) variant is the escape hatch — it
/// forwards a **native tool definition verbatim**, so a caller can drive a
/// server tool Loom does not model yet without waiting for a Loom release.
///
/// # Serde representation
///
/// Serialized as an internally tagged enum with a `"kind"` field (a Loom-owned
/// discriminator, distinct from any provider-native `"type"` field a
/// [`Raw`](ServerTool::Raw) payload carries), rendered in `snake_case`:
///
/// ```json
/// { "kind": "web_search", "max_uses": 5 }
/// { "kind": "code_execution" }
/// { "kind": "raw", "type": "web_search_20250305", "name": "web_search" }
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServerTool {
    /// Provider-hosted web search.
    WebSearch {
        /// The maximum number of searches the model may run this turn. Absent
        /// for the provider default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_uses: Option<u32>,
        /// If set, restricts searches to these domains. Mutually exclusive with
        /// [`blocked_domains`](ServerTool::WebSearch::blocked_domains).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allowed_domains: Option<Vec<String>>,
        /// If set, excludes these domains from search results.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocked_domains: Option<Vec<String>>,
    },

    /// Provider-hosted code execution in a sandboxed container.
    CodeExecution {},

    /// A native server-tool definition forwarded **verbatim**.
    ///
    /// The wrapped value is emitted into the provider request's native tool
    /// array unchanged, so callers can drive a server tool Loom does not yet
    /// model. The value must serialize as a JSON object (a native tool
    /// definition), and must not carry a `"kind"` field of its own.
    Raw(serde_json::Value),
}
