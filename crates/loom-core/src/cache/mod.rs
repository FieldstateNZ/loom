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

mod hint;
mod negotiation;
mod ttl;

pub use hint::CacheHint;
pub use negotiation::CacheNegotiation;
pub use ttl::CacheTtl;
