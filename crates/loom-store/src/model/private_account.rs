//! Initial, immutable private-account provisioning inputs and outcomes.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Server-validated initial account request. Contains no plaintext key.
#[derive(Clone, Debug)]
pub struct NewPrivateAccount {
    /// Existing Tandem issuer/subject namespace, preserving its 160-bit digest.
    pub namespace: String,
    /// Original operator operation, persisted before the request is sent.
    pub operation_id: Uuid,
    /// Digest of the exact versioned identity and proposed key hash.
    pub request_hash: String,
    /// Existing gateway HMAC key lookup hash.
    pub key_hash: String,
    /// Existing non-secret display prefix.
    pub key_prefix: String,
}

/// Committed identifiers; never includes key material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateAccount {
    /// Immutable account namespace.
    pub namespace: String,
    /// Original provisioning operation.
    pub operation_id: Uuid,
    /// Tenant created by this operation.
    pub tenant_id: Uuid,
    /// Key created atomically with the tenant.
    pub key_id: Uuid,
    /// Original commit timestamp.
    pub created_at: DateTime<Utc>,
    /// Whether this call created the rows rather than replaying them.
    pub created: bool,
}
