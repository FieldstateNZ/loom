//! Provider credential storage:
//! `PUT /admin/tenants/{id}/credentials/{provider}`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loom_store::{CredentialStore, NewProviderCredential, TenantStore};

use crate::error::ApiError;
use crate::extract;
use crate::provider::credential_aad;
use crate::state::AppState;

/// Request body for storing a provider credential.
#[derive(Debug, Deserialize)]
pub(super) struct PutCredentialRequest {
    /// The provider API key to store (encrypted at rest).
    api_key: String,
    /// An optional provider base URL override.
    #[serde(default)]
    base_url: Option<String>,
}

/// Response describing a stored credential — never includes the secret.
#[derive(Debug, Serialize)]
struct CredentialResponse {
    /// The credential's unique identifier.
    id: Uuid,
    /// The owning tenant.
    tenant_id: Option<Uuid>,
    /// The provider the credential authenticates against.
    provider: String,
    /// The optional base URL override.
    base_url: Option<String>,
    /// When the credential was created.
    created_at: DateTime<Utc>,
}

/// `PUT /admin/tenants/{id}/credentials/{provider}` — store an encrypted
/// provider credential for a tenant.
pub(super) async fn put_credential(
    State(state): State<AppState>,
    Path((tenant_id, provider)): Path<(Uuid, String)>,
    extract::Json(req): extract::Json<PutCredentialRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.api_key.trim().is_empty() {
        return Err(ApiError::bad_request("api_key must not be empty"));
    }

    // Validate the tenant exists so a bad id yields 404 rather than a FK 500.
    if state
        .store()
        .get_tenant(tenant_id)
        .await
        .map_err(ApiError::from_store)?
        .is_none()
    {
        return Err(ApiError::not_found("tenant not found"));
    }

    // Bind the ciphertext to this row's identity so it cannot be relocated into
    // another tenant's (or provider's) row and still decrypt.
    let aad = credential_aad(Some(tenant_id), &provider);
    let sealed = state
        .crypto()
        .encrypt(req.api_key.as_bytes(), aad.as_bytes())?;

    let stored = state
        .store()
        .upsert_credential(NewProviderCredential {
            tenant_id: Some(tenant_id),
            provider,
            encrypted_secret: sealed.ciphertext,
            nonce: Some(sealed.nonce),
            aad: None,
            base_url: req.base_url,
        })
        .await
        .map_err(ApiError::from_store)?;

    Ok(Json(CredentialResponse {
        id: stored.id,
        tenant_id: stored.tenant_id,
        provider: stored.provider,
        base_url: stored.base_url,
        created_at: stored.created_at,
    }))
}
