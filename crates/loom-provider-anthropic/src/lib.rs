//! `loom-provider-anthropic` — the Anthropic provider implementation.
//!
//! Translates the fluent conversation to Anthropic's native Messages API and
//! back, losslessly, preserving managed capabilities (prompt caching,
//! server-side tools, the MCP connector, extended thinking, batches).
//!
//! > **Scaffold.** Messages API translation lands in issue #4; streaming in #5.
#![forbid(unsafe_code)]

/// Re-export of the fluent conversation domain model.
pub use loom_core;
/// Re-export of the provider trait this crate implements.
pub use loom_provider;
