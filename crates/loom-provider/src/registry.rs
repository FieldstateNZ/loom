//! A registry of providers keyed by name.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::provider::Provider;

/// A registry mapping provider names to their implementations.
///
/// Registration is static in the common case — providers are registered at
/// startup — but the design deliberately keeps room for dynamic loading later:
/// providers are held behind [`Arc`] `dyn` handles, so a future loader could
/// register (or replace) providers at runtime without changing this API.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<String, Arc<dyn Provider>>,
}

impl ProviderRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            providers: BTreeMap::new(),
        }
    }

    /// Registers `provider` under the name reported by its descriptor,
    /// returning any provider previously registered under that name.
    pub fn register(&mut self, provider: Arc<dyn Provider>) -> Option<Arc<dyn Provider>> {
        let name = provider.descriptor().name;
        self.providers.insert(name, provider)
    }

    /// Looks up a provider by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&dyn Provider> {
        self.providers.get(name).map(|p| p.as_ref())
    }

    /// Looks up a provider by name, returning a cloneable shared handle.
    #[must_use]
    pub fn get_shared(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(name).cloned()
    }

    /// Returns `true` if a provider is registered under `name`.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.providers.contains_key(name)
    }

    /// Iterates over the registered provider names in sorted order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.providers.keys().map(String::as_str)
    }

    /// The number of registered providers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Returns `true` if no providers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}
