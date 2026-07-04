//! Translation between Loom's fluent conversation model and Anthropic's native
//! Messages API wire format.
//!
//! The translation is **lossless and provider-faithful** in both directions:
//!
//! - [`translate_request`] maps a [`Conversation`] plus its request-time
//!   [`ConversationOptions`] to the native `POST /v1/messages` request body,
//!   mapping each [`ContentPart`] to its native content block (the inverse of
//!   [`translate_response`]), carrying `thinking` / `redacted_thinking` blocks
//!   through unchanged for multi-turn correctness, and merging the
//!   `provider_options["anthropic"]` bag over the request so callers can pass
//!   any native field (`tool_choice`, `top_p`, thinking config, beta flags, …)
//!   without a Loom release.
//! - [`translate_response`] maps a native Messages response back to an
//!   assistant [`Message`], mapping content blocks to [`ContentPart`]s — with
//!   **unknown block types preserved verbatim** via
//!   [`ContentPart::ProviderExtension`], never an error — and populating
//!   [`Message::raw`] with the verbatim native response for audit and replay.
//!
//! # Prompt caching
//!
//! A provider-agnostic [`CacheHint`] on a cacheable [`ContentPart`], a
//! [`ToolDefinition`], or the conversation's system prompt maps to Anthropic's
//! native `cache_control: { "type": "ephemeral"[, "ttl": "1h"] }` marker on the
//! corresponding native block, and is read back off the block on response
//! translation. When [`ConversationOptions::auto_cache`] is set, Loom
//! additionally places up to two deterministic breakpoints — after the stable
//! system-plus-tools head and on the trailing history boundary — respecting
//! Anthropic's maximum of four cache breakpoints per request. See
//! `apply_auto_cache` (private, in the `cache_control` submodule) and
//! [`strip_cache_control`].
//!
//! These functions are pure and free of I/O, so they can be exercised directly
//! against recorded fixtures.
//!
//! # Module layout
//!
//! The translation surface is split into cohesive submodules: [`request`] (the
//! `Conversation` → native request body direction), [`response`] (the native
//! response → [`Message`] direction), [`server_tools`] (native server-tool and
//! MCP connector entries), [`betas`] (the `anthropic-beta` token set), and
//! [`cache_control`] (prompt-cache markers and auto-cache breakpoints).

mod betas;
mod cache_control;
mod content_block;
mod request;
mod response;
mod server_tools;
mod usage;

pub use betas::required_betas;
pub use cache_control::{requests_caching, strip_cache_control};
pub use request::translate_request;
pub use response::{block_to_part, stop_reason, translate_response};
pub use usage::translate_usage;
