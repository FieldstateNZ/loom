//! Request DTOs for the `/v1` conversation and turn endpoints.

use serde::Deserialize;
use utoipa::ToSchema;

use loom_core::{CacheHint, ConversationOptions, Message};

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
    #[schema(value_type = Object, nullable)]
    pub metadata: Option<serde_json::Value>,
}

/// Request body for appending a turn to a stored conversation.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TurnRequest {
    /// The user turn's content parts, in provider-significant order.
    #[schema(value_type = Vec<Object>)]
    pub content: Vec<loom_core::ContentPart>,
    /// Whether to stream the assistant turn as Server-Sent Events.
    #[serde(default)]
    pub stream: bool,
    /// Request-time provider options (sampling, tools, …).
    #[serde(default)]
    #[schema(value_type = Object, nullable)]
    pub options: Option<ConversationOptions>,
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
    #[schema(value_type = Object, nullable)]
    pub system_cache: Option<CacheHint>,
    /// The full, inline message history to run.
    #[schema(value_type = Vec<Object>)]
    pub messages: Vec<Message>,
    /// Request-time provider options.
    #[serde(default)]
    #[schema(value_type = Object, nullable)]
    pub options: Option<ConversationOptions>,
    /// Whether to stream the assistant turn as Server-Sent Events.
    #[serde(default)]
    pub stream: bool,
}
