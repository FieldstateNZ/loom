//! The static catalogue of Anthropic Claude models and their capabilities.
//!
//! This is **product data** — a snapshot of the models Loom knows how to route
//! to Anthropic, each annotated with the [`Capability`] values it supports.
//! The bound model is resolved against this list before a request is
//! dispatched, and capability negotiation is checked against the matching
//! [`ModelDescriptor`]. The list is intentionally conservative and easy to
//! extend: new models are added here without any translation changes.

mod beta;

use loom_provider::{Capability, ModelDescriptor};

pub use beta::{feature_beta, BetaFeature};

/// The registry name this provider is known by.
pub const PROVIDER_NAME: &str = "anthropic";

/// The capability set shared by the current generation of Claude models.
///
/// Every model in the catalogue below streams, calls client tools, reasons
/// ("thinking"), accepts images and documents, supports prompt caching and
/// batch processing, and can drive Anthropic's server-side web search and code
/// execution tools as well as the MCP connector.
const MODERN_CAPABILITIES: [Capability; 10] = [
    Capability::Streaming,
    Capability::ClientTools,
    Capability::ServerToolWebSearch,
    Capability::ServerToolCodeExecution,
    Capability::McpConnector,
    Capability::PromptCaching,
    Capability::Batches,
    Capability::Thinking,
    Capability::Vision,
    Capability::Documents,
];

/// The Claude model identifiers this provider declares, newest first.
///
/// Includes the current family (`claude-opus-4-8`, `claude-sonnet-5`,
/// `claude-haiku-4-5-20251001`) plus the two most widely used prior models
/// (`claude-opus-4-7`, `claude-sonnet-4-6`) so callers pinned to them keep
/// routing.
const MODEL_IDS: [&str; 5] = [
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
    "claude-haiku-4-5-20251001",
];

/// Returns the static Anthropic model catalogue.
///
/// Each entry is a [`ModelDescriptor`] declaring the capabilities Loom will
/// enforce for that model during capability negotiation.
#[must_use]
pub fn catalogue() -> Vec<ModelDescriptor> {
    MODEL_IDS
        .iter()
        .map(|id| ModelDescriptor::new(*id, MODERN_CAPABILITIES))
        .collect()
}
