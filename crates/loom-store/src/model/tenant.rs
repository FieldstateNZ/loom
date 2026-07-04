//! The tenant row and its insertion type.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A persisted tenant — the unit of multi-tenant isolation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenant {
    /// The tenant's unique identifier.
    pub id: Uuid,
    /// A stable, URL-safe unique handle for the tenant.
    pub slug: String,
    /// A human-readable display name.
    pub name: String,
    /// Lifecycle status (e.g. `"active"`, `"suspended"`).
    pub status: String,
    /// When the tenant was created.
    pub created_at: DateTime<Utc>,
}

/// The fields required to create a [`Tenant`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTenant {
    /// A stable, URL-safe unique handle for the tenant.
    pub slug: String,
    /// A human-readable display name.
    pub name: String,
    /// Lifecycle status. Use `"active"` for a normal tenant.
    pub status: String,
}

impl NewTenant {
    /// Constructs a new tenant description with `status` defaulted to
    /// `"active"`.
    #[must_use]
    pub fn new(slug: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            slug: slug.into(),
            name: name.into(),
            status: "active".to_owned(),
        }
    }
}
