//! Provider resolution: mapping a `(tenant, provider name)` pair to a concrete
//! [`Provider`] instance bound to the tenant's decrypted credential.
//!
//! The gateway never hard-codes a single provider into its handlers. Instead a
//! [`ProviderFactory`] resolves the bound provider at request time, so the
//! conversation API stays a thin layer over the [`Provider`] trait. The
//! [`DefaultProviderFactory`] compiles in the providers Loom ships with — today,
//! Anthropic — loading each tenant's encrypted credential from the store,
//! decrypting it with the gateway [`Crypto`](crate::crypto::Crypto), and
//! constructing a provider bound to that API key (and optional base-URL
//! override). Tests substitute their own factory (for example one returning a
//! `MockProvider`) via [`AppState::with_provider_factory`].

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use loom_provider::Provider;
use loom_provider_anthropic::{AnthropicProvider, PROVIDER_NAME as ANTHROPIC_PROVIDER};
use loom_store::{CredentialStore, ProviderCredential};

use crate::error::ApiError;
use crate::state::AppState;

/// Resolves the [`Provider`] a conversation is bound to.
///
/// Implementations map a provider name (and the calling tenant) to a ready-to-use
/// provider. Credential loading and decryption happen here so handlers never
/// touch secrets directly.
#[async_trait]
pub trait ProviderFactory: Send + Sync {
    /// Resolves `provider` for `tenant_id`, returning a shared provider handle.
    ///
    /// # Errors
    ///
    /// Returns a structured [`ApiError`] — never a panic — when the provider is
    /// unknown, no credential is configured, or a stored credential cannot be
    /// decrypted.
    async fn provider(
        &self,
        state: &AppState,
        tenant_id: Uuid,
        provider: &str,
    ) -> Result<Arc<dyn Provider>, ApiError>;
}

/// The default factory over the providers compiled into the gateway.
///
/// Recognises `"anthropic"`, building an [`AnthropicProvider`] from the tenant's
/// stored credential (falling back to a gateway-global credential). Any other
/// provider name yields a `422` rather than a panic.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultProviderFactory;

#[async_trait]
impl ProviderFactory for DefaultProviderFactory {
    async fn provider(
        &self,
        state: &AppState,
        tenant_id: Uuid,
        provider: &str,
    ) -> Result<Arc<dyn Provider>, ApiError> {
        match provider {
            ANTHROPIC_PROVIDER => {
                let credential = load_credential(state, tenant_id, provider).await?;
                let api_key = decrypt_api_key(state, &credential)?;
                let mut anthropic =
                    AnthropicProvider::new(api_key).map_err(ApiError::from_provider)?;
                if let Some(base_url) = credential.base_url {
                    anthropic = anthropic.with_base_url(base_url);
                }
                Ok(Arc::new(anthropic))
            }
            other => Err(ApiError::unprocessable(
                "unknown_provider",
                format!("provider {other:?} is not available on this gateway"),
            )),
        }
    }
}

/// Loads the credential for `(tenant_id, provider)`, falling back to the
/// gateway-global credential, or fails with a structured `422`.
async fn load_credential(
    state: &AppState,
    tenant_id: Uuid,
    provider: &str,
) -> Result<ProviderCredential, ApiError> {
    if let Some(credential) = state
        .store()
        .get_credential(Some(tenant_id), provider)
        .await
        .map_err(ApiError::from_store)?
    {
        return Ok(credential);
    }
    if let Some(credential) = state
        .store()
        .get_credential(None, provider)
        .await
        .map_err(ApiError::from_store)?
    {
        return Ok(credential);
    }
    Err(ApiError::unprocessable(
        "provider_not_configured",
        format!("no credential is configured for provider {provider:?}"),
    ))
}

/// Decrypts a stored credential's ciphertext back to the plaintext API key.
///
/// The AEAD associated data is rebuilt from the loaded row's own identity via
/// [`credential_aad`], so a ciphertext that was relocated into a different
/// `(tenant, provider)` row fails to decrypt rather than being silently used.
fn decrypt_api_key(state: &AppState, credential: &ProviderCredential) -> Result<String, ApiError> {
    let nonce = credential.nonce.as_deref().ok_or_else(|| {
        tracing::error!("stored credential is missing its encryption nonce");
        ApiError::internal()
    })?;
    let aad = credential_aad(credential.tenant_id, &credential.provider);
    let plaintext = state
        .crypto()
        .decrypt(nonce, &credential.encrypted_secret, aad.as_bytes())?;
    String::from_utf8(plaintext).map_err(|_| {
        tracing::error!("decrypted credential is not valid UTF-8");
        ApiError::internal()
    })
}

/// Builds the AEAD associated data that binds a provider credential's ciphertext
/// to the identity of the row it belongs to.
///
/// The value is `"{tenant_id}:{provider}"` for a tenant-scoped credential, or
/// `":{provider}"` for a gateway-global one (`tenant_id = None`). Both the
/// encrypt path (admin `put_credential`) and the decrypt path (provider
/// resolution) derive it the same way, so a confused-deputy row swap — moving
/// one row's ciphertext into another — yields a mismatched `aad` and fails
/// closed.
pub(crate) fn credential_aad(tenant_id: Option<Uuid>, provider: &str) -> String {
    match tenant_id {
        Some(tenant_id) => format!("{tenant_id}:{provider}"),
        None => format!(":{provider}"),
    }
}
