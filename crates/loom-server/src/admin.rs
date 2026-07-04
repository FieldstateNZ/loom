//! The `/admin` API: tenant, virtual-key and credential provisioning.
//!
//! Every route here is guarded by [`admin_auth`](crate::auth::admin_auth), so
//! handlers assume the caller holds the root admin token. Responses use the
//! shared [`ApiError`] envelope on failure.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use http::StatusCode;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loom_store::{
    CredentialStore, KeyStore, NewProviderCredential, NewTenant, NewVirtualKey, TenantStore,
    UsageStore,
};

use crate::error::ApiError;
use crate::extract;
use crate::keys::{generate_key, KeyEnv};
use crate::provider::credential_aad;
use crate::state::AppState;

/// Builds the `/admin` sub-router (without its guard layer, which the top-level
/// router applies).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/tenants", post(create_tenant))
        .route("/admin/tenants/{id}", get(get_tenant))
        .route("/admin/tenants/{id}/keys", post(create_key))
        .route("/admin/keys/{id}", delete(revoke_key))
        .route(
            "/admin/tenants/{id}/credentials/{provider}",
            put(put_credential),
        )
        .route("/admin/usage", get(usage_by_tenant))
}

/// Request body for creating a tenant.
#[derive(Debug, Deserialize)]
struct CreateTenantRequest {
    /// A stable, URL-safe unique handle for the tenant.
    slug: String,
    /// A human-readable display name.
    name: String,
}

/// `POST /admin/tenants` — create a tenant.
async fn create_tenant(
    State(state): State<AppState>,
    extract::Json(req): extract::Json<CreateTenantRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.slug.trim().is_empty() || req.name.trim().is_empty() {
        return Err(ApiError::bad_request("slug and name must not be empty"));
    }
    let tenant = state
        .store()
        .create_tenant(NewTenant::new(req.slug, req.name))
        .await
        .map_err(ApiError::from_store)?;
    Ok((StatusCode::CREATED, Json(tenant)))
}

/// `GET /admin/tenants/{id}` — fetch a tenant by id.
async fn get_tenant(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant = state
        .store()
        .get_tenant(id)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::not_found("tenant not found"))?;
    Ok(Json(tenant))
}

/// Request body for minting a virtual key.
#[derive(Debug, Deserialize)]
struct CreateKeyRequest {
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
async fn create_key(
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
async fn revoke_key(
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

/// Request body for storing a provider credential.
#[derive(Debug, Deserialize)]
struct PutCredentialRequest {
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
async fn put_credential(
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

/// Query parameters for the gateway-wide usage rollup.
#[derive(Debug, Deserialize)]
struct AdminUsageQuery {
    /// Inclusive lower bound on event time (RFC 3339); open if omitted.
    from: Option<DateTime<Utc>>,
    /// Inclusive upper bound on event time (RFC 3339); open if omitted.
    to: Option<DateTime<Utc>>,
    /// Grouping dimension; only `tenant` is supported gateway-wide.
    group_by: Option<String>,
}

/// One tenant's aggregate usage in the gateway-wide rollup.
#[derive(Debug, Serialize)]
struct AdminUsageRow {
    /// The tenant id.
    group: Option<String>,
    event_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    cost: Decimal,
}

/// The gateway-wide usage response envelope.
#[derive(Debug, Serialize)]
struct AdminUsageResponse {
    group_by: &'static str,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    rows: Vec<AdminUsageRow>,
}

/// `GET /admin/usage?group_by=tenant` — gateway-wide usage rolled up by tenant,
/// over an optional `[from, to]` window. Root-token only.
async fn usage_by_tenant(
    State(state): State<AppState>,
    Query(query): Query<AdminUsageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    // Gateway-wide reporting groups by tenant; reject any other dimension.
    if let Some(group_by) = query.group_by.as_deref() {
        if group_by != "tenant" {
            return Err(ApiError::bad_request(format!(
                "unknown group_by {group_by:?}; only 'tenant' is supported here"
            )));
        }
    }
    let rows = state
        .store()
        .rollup_by_tenant(query.from, query.to)
        .await
        .map_err(ApiError::from_store)?;
    Ok(Json(AdminUsageResponse {
        group_by: "tenant",
        from: query.from,
        to: query.to,
        rows: rows
            .into_iter()
            .map(|r| AdminUsageRow {
                group: r.group,
                event_count: r.event_count,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cache_read_tokens: r.cache_read_tokens,
                cache_write_tokens: r.cache_write_tokens,
                cost: r.cost,
            })
            .collect(),
    }))
}
