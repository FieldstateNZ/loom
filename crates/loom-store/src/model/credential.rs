//! A persisted provider credential and its insertion type.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A persisted provider credential.
///
/// A `None` [`tenant_id`](Self::tenant_id) denotes a gateway-global credential
/// shared by all tenants that do not supply their own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCredential {
    /// The credential's unique identifier.
    pub id: Uuid,
    /// The owning tenant, or `None` for a gateway-global credential.
    pub tenant_id: Option<Uuid>,
    /// The provider this credential authenticates against (e.g. `"anthropic"`).
    pub provider: String,
    /// The encrypted secret bytes (ciphertext).
    pub encrypted_secret: Vec<u8>,
    /// The AEAD nonce used to encrypt the secret, if applicable.
    pub nonce: Option<Vec<u8>>,
    /// The additional authenticated data bound to the ciphertext, if any.
    pub aad: Option<Vec<u8>>,
    /// An optional provider base URL override.
    pub base_url: Option<String>,
    /// When the credential was created.
    pub created_at: DateTime<Utc>,
}

/// The fields required to create (or replace) a [`ProviderCredential`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewProviderCredential {
    /// The owning tenant, or `None` for a gateway-global credential.
    pub tenant_id: Option<Uuid>,
    /// The provider this credential authenticates against.
    pub provider: String,
    /// The encrypted secret bytes (ciphertext).
    pub encrypted_secret: Vec<u8>,
    /// The AEAD nonce used to encrypt the secret, if applicable.
    pub nonce: Option<Vec<u8>>,
    /// The additional authenticated data bound to the ciphertext, if any.
    pub aad: Option<Vec<u8>>,
    /// An optional provider base URL override.
    pub base_url: Option<String>,
}
