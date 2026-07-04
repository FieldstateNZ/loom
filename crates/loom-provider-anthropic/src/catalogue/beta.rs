//! The [`BetaFeature`] enum and its catalogue-driven default
//! `anthropic-beta` token.

/// A provider feature that may require an `anthropic-beta` request header.
///
/// The mapping from a feature to its beta token is **product data** (see
/// [`feature_beta`]), kept next to the model catalogue so that adopting a new
/// beta is a data edit — and, for callers, no edit at all (they can override or
/// add betas per request; see
/// [`translate::required_betas`](crate::translate::required_betas)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BetaFeature {
    /// The provider-hosted web search server tool.
    WebSearch,
    /// The provider-hosted code execution server tool.
    CodeExecution,
    /// The MCP connector (attaching external MCP servers to a request).
    McpConnector,
}

/// The `anthropic-beta` token a feature requires by default, or `None` when the
/// feature is generally available and needs no beta header.
///
/// This is the catalogue-driven default; a caller can override or supplement it
/// per request without a Loom release (see
/// [`translate::required_betas`](crate::translate::required_betas)).
#[must_use]
pub fn feature_beta(feature: BetaFeature) -> Option<&'static str> {
    match feature {
        // Web search is generally available and needs no beta header.
        BetaFeature::WebSearch => None,
        // Code execution ships behind a dated beta flag.
        BetaFeature::CodeExecution => Some("code-execution-2025-05-22"),
        // The MCP connector ships behind a dated beta flag.
        BetaFeature::McpConnector => Some("mcp-client-2025-04-04"),
    }
}
