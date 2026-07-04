//! A provider's self-description, [`ProviderDescriptor`].

use serde::{Deserialize, Serialize};

use super::model_descriptor::ModelDescriptor;

/// A provider's self-description: its name, the models it exposes, and whether
/// it can discover further models dynamically.
///
/// The model list is static in the common case; `dynamic_discovery` records
/// (conceptually, for now) that a provider may enumerate additional models at
/// runtime. Even a dynamic provider is expected to describe the models it
/// already knows about here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    /// The provider's registry name (e.g. `"anthropic"`).
    pub name: String,
    /// The models this provider statically declares.
    pub models: Vec<ModelDescriptor>,
    /// Whether the provider can discover additional models at runtime beyond
    /// those listed in [`models`](ProviderDescriptor::models).
    pub dynamic_discovery: bool,
}

impl ProviderDescriptor {
    /// Creates a descriptor for a provider with a static model list.
    pub fn new(name: impl Into<String>, models: impl IntoIterator<Item = ModelDescriptor>) -> Self {
        Self {
            name: name.into(),
            models: models.into_iter().collect(),
            dynamic_discovery: false,
        }
    }

    /// Marks this provider as capable of dynamic model discovery.
    #[must_use]
    pub fn with_dynamic_discovery(mut self) -> Self {
        self.dynamic_discovery = true;
        self
    }

    /// Looks up a declared model by its identifier.
    #[must_use]
    pub fn model(&self, id: &str) -> Option<&ModelDescriptor> {
        self.models.iter().find(|m| m.id == id)
    }
}
