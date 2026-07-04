//! Provider-agnostic prompt-cache hints.
//!
//! Prompt caching lets a provider reuse the work of processing a stable request
//! prefix across requests, billing the cached span at a reduced rate. Loom
//! models a cache breakpoint provider-agnostically as a [`CacheHint`]: attach
//! one to a cacheable [`ContentPart`](crate::ContentPart), a
//! [`ToolDefinition`](crate::ToolDefinition), or a conversation's system prompt
//! to mark a prefix boundary. Provider translators map the hint to the
//! provider's native marker (for Anthropic, `cache_control`).
//!
//! Hints are **advisory**: a model that does not support prompt caching
//! soft-ignores them by default rather than failing the request — see
//! [`CacheNegotiation`].

use serde::{Deserialize, Serialize};

/// How long a provider should keep a cache entry alive.
///
/// Anthropic exposes exactly these two ephemeral time-to-live tiers; the names
/// are provider-agnostic so other providers can map onto them. Serialized in
/// `snake_case` (`five_minutes`, `one_hour`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CacheTtl {
    /// A short-lived cache entry — Anthropic's default five-minute tier.
    FiveMinutes,
    /// A longer-lived cache entry — Anthropic's one-hour tier.
    OneHour,
}

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

/// How a provider should treat a cache hint on a model that does not declare
/// prompt-caching support.
///
/// Cache hints are advisory, so the default is [`CacheNegotiation::SoftIgnore`]:
/// the hint is stripped (and a warning surfaced via a log and/or response
/// header) rather than failing the request. Callers that would rather learn
/// about the mismatch loudly can opt into [`CacheNegotiation::HardFail`].
///
/// Serialized in `snake_case` (`soft_ignore`, `hard_fail`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CacheNegotiation {
    /// Strip cache hints and continue, surfacing a warning. The default.
    #[default]
    SoftIgnore,
    /// Fail the request with a capability error.
    HardFail,
}
