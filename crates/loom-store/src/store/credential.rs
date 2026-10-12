//! Persistence for provider credentials.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::model::{NewProviderCredential, ProviderCredential};

/// Persistence for provider credentials.
// `async_trait` (<= 0.1.89) stamps a bare `#[must_use]` on each async trait
// method while the boxed future it returns is already `#[must_use]`, which
// trips `clippy::double_must_use` on newer toolchains. The attribute is
// macro-generated, so this scoped allow is the narrowest place to quiet it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait CredentialStore {
    /// Inserts a credential, or replaces the existing one for the same
    /// `(tenant_id, provider)` pair. Returns the persisted row.
    async fn upsert_credential(&self, new: NewProviderCredential) -> Result<ProviderCredential>;

    /// Fetches the credential for a `(tenant_id, provider)` pair. Pass
    /// `tenant_id = None` to fetch the gateway-global credential.
    async fn get_credential(
        &self,
        tenant_id: Option<Uuid>,
        provider: &str,
    ) -> Result<Option<ProviderCredential>>;

    /// Lists all credentials owned by a tenant (or all global credentials when
    /// `tenant_id` is `None`).
    async fn list_credentials(&self, tenant_id: Option<Uuid>) -> Result<Vec<ProviderCredential>>;
}
