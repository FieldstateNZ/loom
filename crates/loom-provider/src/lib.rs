//! `loom-provider` — the provider plugin trait and capability model.
//!
//! Providers are pluggable libraries. Each declares its capabilities and owns
//! translation between the fluent conversation and its native wire protocol; the
//! gateway core never special-cases a provider.
//!
//! > **Scaffold.** The `Provider` trait, `Capability` model, `TurnEvent`
//! > envelope and registry land in issue #3.
#![forbid(unsafe_code)]

/// Re-export of the domain model every provider translates to and from.
pub use loom_core;
