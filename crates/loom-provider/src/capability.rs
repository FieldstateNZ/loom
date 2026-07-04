//! Capability model and capability negotiation.
//!
//! Every provider declares, per model, which [`Capability`] values it supports.
//! Before a request is dispatched, Loom computes the set of capabilities the
//! request actually exercises and checks them against the bound model. If the
//! model does not support a required capability the request fails **fast** with
//! [`ProviderError::CapabilityUnsupported`] — Loom never silently degrades a
//! request by dropping a feature the caller asked for.

use std::collections::BTreeSet;

use loom_core::{ContentPart, Conversation, ConversationOptions, ServerTool};
use serde::{Deserialize, Serialize};

use crate::error::ProviderError;

/// A discrete provider feature that a request may require and a model may
/// support.
///
/// The enum is `#[non_exhaustive]`: providers gain capabilities over time and
/// new variants will be added without a breaking change, so downstream `match`
/// expressions must include a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Capability {
    /// Incremental (server-sent-event style) streaming of a turn.
    Streaming,
    /// Client-defined tools that the model may call (function calling).
    ClientTools,
    /// Provider-hosted web search tool.
    ServerToolWebSearch,
    /// Provider-hosted code execution tool.
    ServerToolCodeExecution,
    /// Connecting the model to external MCP servers via a provider connector.
    McpConnector,
    /// Prompt caching / cache-control markers.
    PromptCaching,
    /// Asynchronous batch processing of requests.
    Batches,
    /// Extended reasoning / "thinking" blocks.
    Thinking,
    /// Image inputs.
    Vision,
    /// Document (e.g. PDF) inputs.
    Documents,
}

/// A single model offered by a provider, together with the capabilities it
/// declares support for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// The provider-native model identifier (e.g. `"claude-opus-4-8"`).
    pub id: String,
    /// The set of capabilities this model supports.
    pub capabilities: BTreeSet<Capability>,
}

impl ModelDescriptor {
    /// Creates a descriptor for `id` declaring the given `capabilities`.
    pub fn new(id: impl Into<String>, capabilities: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            id: id.into(),
            capabilities: capabilities.into_iter().collect(),
        }
    }

    /// Returns `true` if this model declares support for `capability`.
    #[must_use]
    pub fn supports(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// A provider's self-description: its name, the models it exposes, and whether
/// it can discover further models dynamically.
///
/// The model list is static in the common case; `dynamic_discovery` records
/// (conceptually, for now) that a provider may enumerate additional models at
/// runtime. Even a dynamic provider is expected to describe the models it
/// already knows about here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    /// The provider's registry name (e.g. `"anthropic"`).
    pub name: String,
    /// The models this provider statically declares.
    pub models: Vec<ModelDescriptor>,
    /// Whether the provider can discover additional models at runtime beyond
    /// those listed in [`models`](ProviderDescriptor::models).
    pub dynamic_discovery: bool,
}

impl ProviderDescriptor {
    /// Creates a descriptor for a provider with a static model list.
    pub fn new(name: impl Into<String>, models: impl IntoIterator<Item = ModelDescriptor>) -> Self {
        Self {
            name: name.into(),
            models: models.into_iter().collect(),
            dynamic_discovery: false,
        }
    }

    /// Marks this provider as capable of dynamic model discovery.
    #[must_use]
    pub fn with_dynamic_discovery(mut self) -> Self {
        self.dynamic_discovery = true;
        self
    }

    /// Looks up a declared model by its identifier.
    #[must_use]
    pub fn model(&self, id: &str) -> Option<&ModelDescriptor> {
        self.models.iter().find(|m| m.id == id)
    }
}

/// Computes the set of capabilities a request exercises.
///
/// This is a best-effort static analysis of the [`Conversation`] and
/// [`ConversationOptions`]: it inspects offered tools and the content parts
/// already present in the conversation (images, documents, reasoning blocks,
/// server-side tool use) and maps them to the capabilities they require. It
/// deliberately does **not** try to infer capabilities that are only knowable
/// from opaque provider option payloads (e.g. cache directives); a provider
/// translator may extend the required set before calling
/// [`ensure_supported`].
#[must_use]
pub fn required_capabilities(
    conversation: &Conversation,
    options: &ConversationOptions,
) -> BTreeSet<Capability> {
    let mut required = BTreeSet::new();

    if !options.tools.is_empty() {
        required.insert(Capability::ClientTools);
    }

    // Server-side tools the caller offers require their provider-hosted
    // capability. A `Raw` passthrough is forward-compat — Loom cannot infer
    // which capability it exercises, so it is left to the provider.
    for tool in &options.server_tools {
        match tool {
            ServerTool::WebSearch { .. } => {
                required.insert(Capability::ServerToolWebSearch);
            }
            ServerTool::CodeExecution { .. } => {
                required.insert(Capability::ServerToolCodeExecution);
            }
            _ => {}
        }
    }

    for message in &conversation.messages {
        for part in &message.content {
            match part {
                ContentPart::Image { .. } => {
                    required.insert(Capability::Vision);
                }
                ContentPart::Document { .. } => {
                    required.insert(Capability::Documents);
                }
                ContentPart::Thinking { .. } | ContentPart::RedactedThinking { .. } => {
                    required.insert(Capability::Thinking);
                }
                ContentPart::ServerToolUse { name, .. } => {
                    if let Some(cap) = server_tool_capability(name) {
                        required.insert(cap);
                    }
                }
                _ => {}
            }
        }
    }

    required
}

/// Maps a provider-hosted server tool name to the capability it requires, if
/// recognised.
fn server_tool_capability(name: &str) -> Option<Capability> {
    match name {
        "web_search" | "web_search_tool" | "web_search_20250305" => {
            Some(Capability::ServerToolWebSearch)
        }
        "code_execution" | "code_execution_20250522" | "bash" => {
            Some(Capability::ServerToolCodeExecution)
        }
        _ => None,
    }
}

/// Fails fast if `model` does not support every capability in `required`.
///
/// Returns [`ProviderError::CapabilityUnsupported`] for the first unsupported
/// capability, naming the offending `provider` and `model` so the caller
/// receives a structured, actionable error rather than a silently degraded
/// response.
pub fn ensure_supported(
    provider: &str,
    model: &ModelDescriptor,
    required: &BTreeSet<Capability>,
) -> Result<(), ProviderError> {
    for &capability in required {
        if !model.supports(capability) {
            return Err(ProviderError::CapabilityUnsupported {
                capability,
                provider: provider.to_owned(),
                model: model.id.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use loom_core::{ProviderBinding, ServerTool};
    use uuid::Uuid;

    fn conversation() -> Conversation {
        Conversation::new(Uuid::new_v4(), ProviderBinding::new("anthropic", "m"))
    }

    #[test]
    fn offered_server_tools_require_their_hosted_capability() {
        let mut options = ConversationOptions::new();
        options.server_tools = vec![
            ServerTool::WebSearch {
                max_uses: None,
                allowed_domains: None,
                blocked_domains: None,
            },
            ServerTool::CodeExecution {},
        ];
        let required = required_capabilities(&conversation(), &options);
        assert!(required.contains(&Capability::ServerToolWebSearch));
        assert!(required.contains(&Capability::ServerToolCodeExecution));
    }

    #[test]
    fn raw_server_tool_requires_no_inferred_capability() {
        let mut options = ConversationOptions::new();
        options.server_tools = vec![ServerTool::Raw(serde_json::json!({
            "type": "web_fetch_20250910",
            "name": "web_fetch"
        }))];
        // Loom cannot infer a Raw passthrough's capability; it is left to the
        // provider rather than blocking the request here.
        assert!(required_capabilities(&conversation(), &options).is_empty());
    }

    #[test]
    fn negotiation_rejects_a_model_missing_a_server_tool_capability() {
        let mut options = ConversationOptions::new();
        options.server_tools = vec![ServerTool::WebSearch {
            max_uses: None,
            allowed_domains: None,
            blocked_domains: None,
        }];
        // A model that streams and calls client tools but hosts no web search.
        let model = ModelDescriptor::new("m", [Capability::ClientTools]);
        let required = required_capabilities(&conversation(), &options);
        let error = ensure_supported("anthropic", &model, &required)
            .expect_err("web search unsupported must fail fast");
        match error {
            ProviderError::CapabilityUnsupported { capability, .. } => {
                assert_eq!(capability, Capability::ServerToolWebSearch);
            }
            other => panic!("expected CapabilityUnsupported, got {other:?}"),
        }
    }
}
