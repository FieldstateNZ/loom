//! A persisted virtual API key and its insertion type.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::budget::Budget;
use super::rate_limit::RateLimit;

/// A persisted virtual API key belonging to a tenant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualKey {
    /// The key's unique identifier.
    pub id: Uuid,
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// A cryptographic hash of the secret key material (never the secret).
    pub key_hash: String,
    /// A short, non-secret prefix used to identify the key in logs and UIs.
    pub key_prefix: String,
    /// A human-readable label.
    pub name: String,
    /// Lifecycle status (e.g. `"active"`, `"revoked"`).
    pub status: String,
    /// The authorisation scopes granted to the key.
    pub scopes: Vec<String>,
    /// An optional spend budget (overrides the tenant default).
    pub budget: Option<Budget>,
    /// An optional per-key rate limit.
    pub rate_limit: Option<RateLimit>,
    /// When the key was created.
    pub created_at: DateTime<Utc>,
    /// When the key was last used to authenticate a request, if ever.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// The fields required to create a [`VirtualKey`].
#[derive(Clone, Debug, PartialEq)]
pub struct NewVirtualKey {
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// A cryptographic hash of the secret key material.
    pub key_hash: String,
    /// A short, non-secret prefix identifying the key.
    pub key_prefix: String,
    /// A human-readable label.
    pub name: String,
    /// The authorisation scopes granted to the key.
    pub scopes: Vec<String>,
    /// An optional spend budget.
    pub budget: Option<Budget>,
}
