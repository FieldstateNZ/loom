//! `loom-core` — Loom's **fluent conversation** domain model.
//!
//! This crate owns the provider-agnostic abstraction that Loom exposes to
//! clients: conversations, messages, content parts, usage and options. Provider
//! libraries translate this model to and from each provider's native wire
//! format without flattening provider-specific capabilities.
//!
//! > **Scaffold.** The domain model itself lands in issue #2; this placeholder
//! > exists so the workspace compiles from the first commit.
#![forbid(unsafe_code)]

/// The crate version, sourced from Cargo at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}
