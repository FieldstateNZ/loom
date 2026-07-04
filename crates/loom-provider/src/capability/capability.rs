//! A discrete provider feature, [`Capability`].

use serde::{Deserialize, Serialize};

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
