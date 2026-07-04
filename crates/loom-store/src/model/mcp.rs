//! A persisted, tenant-scoped MCP server registration and its insertion type.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A persisted, tenant-scoped MCP server registration.
///
/// Conversations reference an MCP server **by name**; the gateway resolves the
/// registration at request time, loads the URL, and decrypts
/// [`encrypted_token`](Self::encrypted_token) to inject the authorization token
/// upstream. The token ciphertext follows the same envelope-encryption pattern
/// as [`ProviderCredential`](crate::ProviderCredential): it is bound (via AEAD
/// associated data) to the `(tenant_id, name)` identity of the row so it cannot
/// be relocated. The plaintext token is never exposed by the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServer {
    /// The registration's unique identifier.
    pub id: Uuid,
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// The tenant-unique logical name conversations reference.
    pub name: String,
    /// The MCP server endpoint URL.
    pub url: String,
    /// The encrypted authorization-token ciphertext, or `None` when the server
    /// needs no authorization.
    pub encrypted_token: Option<Vec<u8>>,
    /// The AEAD nonce used to encrypt the token, present iff
    /// [`encrypted_token`](Self::encrypted_token) is.
    pub nonce: Option<Vec<u8>>,
    /// An optional provider-native tool-configuration object (e.g. a tool
    /// allow-list), forwarded verbatim into the request.
    pub tool_configuration: Option<serde_json::Value>,
    /// When the registration was created.
    pub created_at: DateTime<Utc>,
    /// When the registration was last updated.
    pub updated_at: DateTime<Utc>,
}

/// The fields required to create (or replace) an [`McpServer`] registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMcpServer {
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// The tenant-unique logical name.
    pub name: String,
    /// The MCP server endpoint URL.
    pub url: String,
    /// The encrypted authorization-token ciphertext, or `None` for no auth.
    pub encrypted_token: Option<Vec<u8>>,
    /// The AEAD nonce, present iff a token ciphertext is.
    pub nonce: Option<Vec<u8>>,
    /// An optional provider-native tool-configuration object.
    pub tool_configuration: Option<serde_json::Value>,
}
