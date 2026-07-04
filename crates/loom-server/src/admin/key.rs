//! Virtual-key provisioning: `POST /admin/tenants/{id}/keys`,
//! `DELETE /admin/keys/{id}`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loom_store::{KeyStore, NewVirtualKey, TenantStore};

use crate::error::ApiError;
use crate::extract;
use crate::keys::{generate_key, KeyEnv};
use crate::state::AppState;

/// Request body for minting a virtual key.
#[derive(Debug, Deserialize)]
pub(super) struct CreateKeyRequest {
    /// A human-readable label for the key.
    name: String,
    /// The environment label (`live` or `test`); defaults to `live`.
    #[serde(default)]
    env: Option<String>,
}

/// Response for a freshly minted virtual key. The plaintext `key` is returned
/// exactly once and never again.
#[derive(Debug, Serialize)]
struct CreateKeyResponse {
    /// The key's unique identifier.
    id: Uuid,
    /// The owning tenant.
    tenant_id: Uuid,
    /// The key's label.
    name: String,
    /// The plaintext secret — shown only here, never stored.
    key: String,
    /// The non-secret display prefix.
    key_prefix: String,
    /// When the key was created.
    created_at: DateTime<Utc>,
}

/// `POST /admin/tenants/{id}/keys` — mint a virtual key for a tenant.
pub(super) async fn create_key(
    State(state): State<AppState>,
    Path(tenant_id): Path<Uuid>,
    extract::Json(req): extract::Json<CreateKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::bad_request("name must not be empty"));
    }
    let env = match req.env.as_deref() {
        None => KeyEnv::Live,
        Some(label) => KeyEnv::parse(label)
            .map_err(|bad| ApiError::bad_request(format!("unknown env label {bad:?}")))?,
    };

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

    let generated = generate_key(env);
    let key_hash = state.hasher().hash(&generated.secret);

    let stored = state
        .store()
        .create_key(NewVirtualKey {
            tenant_id,
            key_hash,
            key_prefix: generated.prefix.clone(),
            name: req.name,
            scopes: Vec::new(),
            budget: None,
        })
        .await
        .map_err(ApiError::from_store)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateKeyResponse {
            id: stored.id,
            tenant_id: stored.tenant_id,
            name: stored.name,
            key: generated.secret,
            key_prefix: stored.key_prefix,
            created_at: stored.created_at,
        }),
    ))
}

/// `DELETE /admin/keys/{id}` — revoke a virtual key.
pub(super) async fn revoke_key(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    if state
        .store()
        .revoke_key(id)
        .await
        .map_err(ApiError::from_store)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("key not found"))
    }
}
