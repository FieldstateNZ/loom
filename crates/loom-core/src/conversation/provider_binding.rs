//! The provider and model a conversation is bound to.

use serde::{Deserialize, Serialize};

/// The provider a conversation is bound to, and the model to use.
///
/// The binding is intentionally a pair of plain strings rather than an
/// enumeration: providers and their model catalogues evolve independently of
/// Loom's release cycle, and Loom must never reject a valid model just because
/// it predates a given build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBinding {
    /// The provider's name (e.g. `"anthropic"`).
    pub provider: String,
    /// The model identifier as the provider expects it (e.g.
    /// `"claude-opus-4-8"`).
    pub model: String,
}

impl ProviderBinding {
    /// Constructs a binding from a provider name and model identifier.
    #[must_use]
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}
