//! A single model's capability declaration, [`ModelDescriptor`].

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::capability::Capability;

/// A single model offered by a provider, together with the capabilities it
/// declares support for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// The provider-native model identifier (e.g. `"claude-opus-4-8"`).
    pub id: String,
    /// The set of capabilities this model supports.
    pub capabilities: BTreeSet<Capability>,
}

impl ModelDescriptor {
    /// Creates a descriptor for `id` declaring the given `capabilities`.
    pub fn new(id: impl Into<String>, capabilities: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            id: id.into(),
            capabilities: capabilities.into_iter().collect(),
        }
    }

    /// Returns `true` if this model declares support for `capability`.
    #[must_use]
    pub fn supports(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}
