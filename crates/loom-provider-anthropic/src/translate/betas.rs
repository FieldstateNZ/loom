//! The `anthropic-beta` request header token set: derived from the request's
//! features and merged with any caller-supplied overrides.

use loom_core::{Conversation, ConversationOptions, ServerTool};
use serde_json::Value;
use std::collections::BTreeSet;

use crate::catalogue::{feature_beta, BetaFeature};

/// The reserved `provider_options["anthropic"]` key carrying caller-supplied
/// `anthropic-beta` tokens. It is consumed for the request header and never
/// merged into the request body.
pub(super) const RESERVED_BETAS_KEY: &str = "betas";

/// Computes the set of `anthropic-beta` tokens a request requires, deterministic
/// and de-duplicated.
///
/// The set is the union of:
///
/// - the catalogue-driven default token for each server-tool feature the
///   request uses (see [`feature_beta`]); and
/// - any tokens the caller supplied verbatim through the reserved
///   `provider_options["anthropic"]["betas"]` array.
///
/// This is the mechanism that lets a new beta be adopted **without a Loom
/// release**: a caller adds the token to `betas` (to add), and can override a
/// stale default by disabling auto-derivation on the provider and supplying the
/// full set. The [`AnthropicProvider`](crate::AnthropicProvider) merges these
/// with any betas configured on the provider itself and sends them as the
/// `anthropic-beta` header.
#[must_use]
pub fn required_betas(_conversation: &Conversation, options: &ConversationOptions) -> Vec<String> {
    let mut betas = BTreeSet::new();

    for tool in &options.server_tools {
        let feature = match tool {
            ServerTool::WebSearch { .. } => Some(BetaFeature::WebSearch),
            ServerTool::CodeExecution { .. } => Some(BetaFeature::CodeExecution),
            // A `Raw` passthrough's beta (if any) is the caller's responsibility
            // via the `betas` override; Loom cannot infer it.
            _ => None,
        };
        if let Some(token) = feature.and_then(feature_beta) {
            betas.insert(token.to_owned());
        }
    }

    // Attaching external MCP servers requires the connector's beta flag.
    if !options.mcp_servers.is_empty() {
        if let Some(token) = feature_beta(BetaFeature::McpConnector) {
            betas.insert(token.to_owned());
        }
    }

    for token in configured_betas(options) {
        betas.insert(token);
    }

    betas.into_iter().collect()
}

/// Reads caller-supplied `anthropic-beta` tokens from the reserved
/// `provider_options["anthropic"]["betas"]` array, ignoring non-string entries.
fn configured_betas(options: &ConversationOptions) -> Vec<String> {
    options
        .provider_options
        .get("anthropic")
        .and_then(|bag| bag.get(RESERVED_BETAS_KEY))
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
