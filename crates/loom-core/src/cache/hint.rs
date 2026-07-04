//! A single prompt-cache breakpoint.

use serde::{Deserialize, Serialize};

use super::CacheTtl;

/// A request to cache the request prefix up to and including the annotated
/// element.
///
/// This is Loom's provider-agnostic spelling of a cache breakpoint (Anthropic's
/// `cache_control: { "type": "ephemeral" }`). A hint is always *ephemeral*;
/// [`ttl`](CacheHint::ttl) optionally selects a non-default lifetime.
///
/// # Serde representation
///
/// Serializes as an object carrying only a non-default `ttl`, so a default hint
/// round-trips as `{}` and a hint with an explicit lifetime as
/// `{ "ttl": "one_hour" }`. This keeps the on-the-wire domain form stable and
/// provider-agnostic; the mapping to a provider's native marker lives in that
/// provider's translator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CacheHint {
    /// The cache lifetime, or `None` for the provider's default (short) TTL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<CacheTtl>,
}

impl CacheHint {
    /// Constructs a cache hint using the provider's default (short) TTL.
    #[must_use]
    pub fn ephemeral() -> Self {
        Self { ttl: None }
    }

    /// Constructs a cache hint with an explicit time-to-live.
    #[must_use]
    pub fn with_ttl(ttl: CacheTtl) -> Self {
        Self { ttl: Some(ttl) }
    }
}
