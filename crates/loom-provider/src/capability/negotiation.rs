//! Capability negotiation: [`required_capabilities`] and [`ensure_supported`].
//!
//! Every provider declares, per model, which [`Capability`] values it supports.
//! Before a request is dispatched, Loom computes the set of capabilities the
//! request actually exercises and checks them against the bound model. If the
//! model does not support a required capability the request fails **fast** with
//! [`ProviderError::CapabilityUnsupported`] — Loom never silently degrades a
//! request by dropping a feature the caller asked for.

use std::collections::BTreeSet;

use loom_core::{ContentPart, Conversation, ConversationOptions, ServerTool};

use super::capability::Capability;
use super::model_descriptor::ModelDescriptor;
use crate::error::ProviderError;

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

    // Offering external MCP servers requires the provider's MCP connector.
    if !options.mcp_servers.is_empty() {
        required.insert(Capability::McpConnector);
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
    fn offered_mcp_servers_require_the_connector_capability() {
        let mut options = ConversationOptions::new();
        options.mcp_servers = vec![loom_core::McpServerRef::named("github")];
        let required = required_capabilities(&conversation(), &options);
        assert!(required.contains(&Capability::McpConnector));
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
