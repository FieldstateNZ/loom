//! The definition of a client-side tool.

use serde::{Deserialize, Serialize};

use crate::CacheHint;

/// The definition of a **client-side** tool the model may choose to call.
///
/// Server-side (provider-executed) tools are configured through the
/// [`provider_options`] bag rather than here, because their configuration is
/// provider-native.
///
/// [`provider_options`]: super::ConversationOptions::provider_options
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The tool's name, as the model will refer to it when calling.
    pub name: String,

    /// A natural-language description of what the tool does, used by the model
    /// to decide when to call it. Absent when the provider allows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// A JSON Schema describing the tool's input arguments.
    pub input_schema: serde_json::Value,

    /// An optional prompt-cache breakpoint on this tool definition.
    ///
    /// Tools render at the head of the request (before the system prompt and
    /// messages), so a breakpoint here caches the tool prefix. Absent (rather
    /// than `null`) when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<CacheHint>,
}
