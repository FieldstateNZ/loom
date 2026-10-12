//! Persistence for tenants.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::model::{NewTenant, Tenant};

/// Persistence for tenants.
// `async_trait` (<= 0.1.89) stamps a bare `#[must_use]` on each async trait
// method while the boxed future it returns is already `#[must_use]`, which
// trips `clippy::double_must_use` on newer toolchains. The attribute is
// macro-generated, so this scoped allow is the narrowest place to quiet it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait TenantStore {
    /// Creates a tenant and returns the persisted row.
    async fn create_tenant(&self, new: NewTenant) -> Result<Tenant>;

    /// Fetches a tenant by id, or `None` if it does not exist.
    async fn get_tenant(&self, id: Uuid) -> Result<Option<Tenant>>;

    /// Fetches a tenant by its unique slug, or `None` if it does not exist.
    async fn get_tenant_by_slug(&self, slug: &str) -> Result<Option<Tenant>>;
}
