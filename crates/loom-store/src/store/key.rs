//! Persistence for virtual API keys.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::model::{NewVirtualKey, VirtualKey};

/// Persistence for virtual API keys.
// `async_trait` (<= 0.1.89) stamps a bare `#[must_use]` on each async trait
// method while the boxed future it returns is already `#[must_use]`, which
// trips `clippy::double_must_use` on newer toolchains. The attribute is
// macro-generated, so this scoped allow is the narrowest place to quiet it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait KeyStore {
    /// Creates a virtual key and returns the persisted row.
    async fn create_key(&self, new: NewVirtualKey) -> Result<VirtualKey>;

    /// Looks a key up by its hash, or `None` if no such key exists.
    ///
    /// This is the hot authentication path.
    async fn get_key_by_hash(&self, key_hash: &str) -> Result<Option<VirtualKey>>;

    /// Marks a key revoked. Returns `true` if a key was updated.
    async fn revoke_key(&self, id: Uuid) -> Result<bool>;

    /// Records that a key was just used, updating its `last_used_at`.
    ///
    /// Returns `true` if a key was updated.
    async fn touch_key_last_used(&self, id: Uuid) -> Result<bool>;
}
