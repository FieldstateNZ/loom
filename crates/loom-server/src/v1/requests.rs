//! Request DTOs for the `/v1` conversation and turn endpoints.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use loom_core::{CacheHint, ConversationOptions, Message};

/// Opt-in private gateway memory execution. Only registered names and a read
/// subset are accepted. The caller's expected identity is an assertion, never
/// permission: it must equal both the operator registration and the peer.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GatewayMcpRequest {
    /// Tenant's operator-registered MCP server name.
    pub name: String,
    /// Requested subset of the registration's finite memory-read policy.
    pub allowed_tools: Vec<String>,
    /// Provisioned identity the application expects. Cannot override the
    /// operator's registered binding or the credential's actual identity.
    pub expected_binding: loom_mcp::Binding,
    /// Offer host-owned task proposals. Never grants permission to dispatch.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub propose_tasks: bool,
    /// Enable scoped saves in the real-world personal assistant profile.
    /// Requires task profile and an independent operator memory-write opt-in.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remember_memories: bool,
    /// Host-owned context provenance. Task proposals require `real_world` in
    /// both this request and the operator registration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
}

/// Request body for creating a conversation.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateConversationRequest {
    /// The provider to bind the conversation to (e.g. `"anthropic"`).
    pub provider: String,
    /// The model identifier, as the provider expects it.
    pub model: String,
    /// An optional system prompt applied to the whole conversation.
    #[serde(default)]
    pub system: Option<String>,
    /// Free-form caller metadata (tags, correlation IDs, …).
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Request body for appending a turn to a stored conversation.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TurnRequest {
    /// The user turn's content parts, in provider-significant order.
    pub content: Vec<loom_core::ContentPart>,
    /// Whether to stream the assistant turn as Server-Sent Events.
    #[serde(default)]
    pub stream: bool,
    /// Request-time provider options (sampling, tools, …).
    #[serde(default)]
    pub options: Option<ConversationOptions>,
    /// Private gateway execution, separate from provider-hosted MCP options.
    #[serde(default)]
    pub gateway_mcp: Option<GatewayMcpRequest>,
}

/// Request body for a stateless turn: the whole conversation is supplied inline
/// and nothing is persisted.
#[derive(Debug, Deserialize, ToSchema)]
pub struct StatelessTurnRequest {
    /// The provider to run against (e.g. `"anthropic"`).
    pub provider: String,
    /// The model identifier, as the provider expects it.
    pub model: String,
    /// An optional system prompt.
    #[serde(default)]
    pub system: Option<String>,
    /// An optional prompt-cache breakpoint on the system prompt.
    #[serde(default)]
    pub system_cache: Option<CacheHint>,
    /// The full, inline message history to run.
    pub messages: Vec<Message>,
    /// Request-time provider options.
    #[serde(default)]
    pub options: Option<ConversationOptions>,
    /// Whether to stream the assistant turn as Server-Sent Events.
    #[serde(default)]
    pub stream: bool,
    /// Private MCP requires a persistent conversation; present values are
    /// explicitly rejected rather than silently ignored on this endpoint.
    #[serde(default)]
    pub gateway_mcp: Option<GatewayMcpRequest>,
}
