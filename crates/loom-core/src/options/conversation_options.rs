//! Provider-agnostic and provider-specific request options.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{McpServerRef, ServerTool, ToolDefinition};
use crate::CacheNegotiation;

/// Options that shape how a provider should generate a response.
///
/// The common, cross-provider sampling controls are modelled as typed fields.
/// Anything provider-specific — Anthropic's `tool_choice` and `top_p`, cache
/// directives, and so on — lives in the [`provider_options`] bag, keyed by
/// provider name. This keeps the common path typed while never forcing a
/// provider feature to be expressed as a stringly-typed hack.
///
/// The type is `#[non_exhaustive]` and implements [`Default`], so callers can
/// build it up field by field:
///
/// ```
/// use loom_core::ConversationOptions;
///
/// let mut opts = ConversationOptions::new();
/// opts.temperature = Some(0.7);
/// opts.max_tokens = Some(1024);
/// assert_eq!(opts.temperature, Some(0.7));
/// ```
///
/// [`provider_options`]: ConversationOptions::provider_options
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[non_exhaustive]
pub struct ConversationOptions {
    /// Sampling temperature. Provider-defined range; typically `0.0..=1.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// The maximum number of tokens the model may generate in its response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,

    /// Sequences that, when generated, cause the model to stop. Empty when
    /// unset.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,

    /// Definitions of the **client-side** tools the model may call. Empty when
    /// no tools are offered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,

    /// Definitions of the **provider-executed** (server-side) tools the model
    /// may use — web search, code execution, and, via
    /// [`ServerTool::Raw`], any native tool Loom does not model yet.
    ///
    /// Unlike [`tools`](ConversationOptions::tools), the host does not execute
    /// these: the provider runs them server-side and returns the results in the
    /// same response (as
    /// [`ServerToolUse`](crate::ContentPart::ServerToolUse) /
    /// [`ServerToolResult`](crate::ContentPart::ServerToolResult) parts). Empty
    /// when none are offered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_tools: Vec<ServerTool>,

    /// Opt-in automatic prompt caching for this request.
    ///
    /// When `true`, a provider translator deterministically places cache
    /// breakpoints for the (typically persisted) conversation — after the
    /// stable system-plus-tools head, and on the trailing history boundary —
    /// without the caller annotating individual blocks. This is the recommended
    /// path for persisted conversations, where per-block hints on reconstructed
    /// history would otherwise have to be re-applied every turn. Defaults to
    /// `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_cache: bool,

    /// How a provider should treat cache hints on a model that does not support
    /// prompt caching. Defaults to [`CacheNegotiation::SoftIgnore`] — cache
    /// hints are advisory.
    #[serde(default, skip_serializing_if = "is_default_negotiation")]
    pub cache_negotiation: CacheNegotiation,

    /// External MCP (Model Context Protocol) servers the provider should
    /// connect to on the model's behalf, so the model can call their tools.
    ///
    /// Each entry references a server either **by name** — resolved by the
    /// gateway against a tenant's registered servers, which injects the URL and
    /// (decrypted) authorization token **server-side** so the secret never
    /// transits the client — or **inline** with an explicit
    /// [`url`](McpServerRef::url) and optional
    /// [`authorization`](McpServerRef::authorization) (the advanced path). Empty
    /// when none are offered. See [`ServerTool`] for provider-*executed* tools;
    /// MCP tools are executed by the connected server, brokered by the provider.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpServerRef>,

    /// A per-provider bag of native options that Loom does not model as typed
    /// fields, keyed by provider name (e.g. `"anthropic"`).
    ///
    /// This is where provider-specific knobs live — `tool_choice`, `top_p`,
    /// `top_k`, thinking configuration, cache directives, and so on — as a
    /// JSON value the provider translator understands. A [`BTreeMap`] is used
    /// so serialization is deterministic. Empty when unset.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_options: BTreeMap<String, serde_json::Value>,
}

/// Whether a cache-negotiation policy is the default, so it can be omitted from
/// the serialized form.
fn is_default_negotiation(value: &CacheNegotiation) -> bool {
    *value == CacheNegotiation::default()
}

impl ConversationOptions {
    /// Constructs an empty set of options with every field at its default.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}
