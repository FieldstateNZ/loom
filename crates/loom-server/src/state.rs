//! Shared application state handed to every request handler.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use loom_store::PgStore;

use crate::config::Config;
use crate::crypto::Crypto;
use crate::keys::KeyHasher;

/// The state shared across all handlers.
///
/// Cheap to clone: everything lives behind a single [`Arc`], so cloning it (as
/// axum does per request) only bumps a reference count.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    store: PgStore,
    crypto: Crypto,
    hasher: KeyHasher,
    root_admin_token: String,
}

impl AppState {
    /// Assembles application state from its parts.
    #[must_use]
    pub fn new(
        store: PgStore,
        crypto: Crypto,
        hasher: KeyHasher,
        root_admin_token: String,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                crypto,
                hasher,
                root_admin_token,
            }),
        }
    }

    /// Assembles application state from a validated [`Config`] and a connected
    /// store, consuming the config's secrets.
    #[must_use]
    pub fn from_config(config: &Config, store: PgStore) -> Self {
        Self::new(
            store,
            Crypto::new(config.encryption_key()),
            KeyHasher::new(config.key_pepper().to_vec()),
            config.root_admin_token().to_owned(),
        )
    }

    /// The persistence layer.
    #[must_use]
    pub fn store(&self) -> &PgStore {
        &self.inner.store
    }

    /// The credential encryptor.
    #[must_use]
    pub fn crypto(&self) -> &Crypto {
        &self.inner.crypto
    }

    /// The virtual-key hasher.
    #[must_use]
    pub fn hasher(&self) -> &KeyHasher {
        &self.inner.hasher
    }

    /// Constant-time comparison of a presented admin token against the
    /// configured root token.
    ///
    /// Both sides are SHA-256 digested first, so the comparison runs over
    /// fixed-length inputs and leaks neither the token length nor an early
    /// mismatch position.
    #[must_use]
    pub fn verify_admin_token(&self, presented: &str) -> bool {
        let presented = Sha256::digest(presented.as_bytes());
        let expected = Sha256::digest(self.inner.root_admin_token.as_bytes());
        presented.ct_eq(&expected).into()
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("store", &self.inner.store)
            .field("crypto", &self.inner.crypto)
            .field("hasher", &self.inner.hasher)
            .field("root_admin_token", &"<redacted>")
            .finish()
    }
}
