//! The `/admin` API: tenant, virtual-key and credential provisioning, plus
//! budget and rate-limit administration.
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
    BudgetStore, CredentialStore, KeyStore, McpServerStore, NewMcpServer, NewProviderCredential,
    NewTenant, NewVirtualKey, RateLimit, TenantStore, UsageStore,
};

use crate::budget::parse_budget;
use crate::error::ApiError;
use crate::extract;
use crate::keys::{generate_key, KeyEnv};
use crate::provider::{credential_aad, mcp_aad};
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
        .route("/admin/tenants/{id}/mcp-servers", get(list_mcp_servers))
        .route(
            "/admin/tenants/{id}/mcp-servers/{name}",
            put(put_mcp_server).delete(delete_mcp_server),
        )
        .route(
            "/admin/tenants/{id}/budget",
            put(put_tenant_budget).delete(delete_tenant_budget),
        )
        .route(
            "/admin/keys/{id}/budget",
            put(put_key_budget).delete(delete_key_budget),
        )
        .route(
            "/admin/keys/{id}/rate-limit",
            put(put_key_rate_limit).delete(delete_key_rate_limit),
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

/// Request body for registering (or replacing) a tenant MCP server.
#[derive(Debug, Deserialize)]
struct PutMcpServerRequest {
    /// The MCP server endpoint URL.
    url: String,
    /// An optional bearer authorization token (encrypted at rest, never
    /// returned).
    #[serde(default)]
    authorization_token: Option<String>,
    /// An optional provider-native tool-configuration object (e.g. an
    /// allow-list of tool names), stored and forwarded verbatim.
    #[serde(default)]
    tool_configuration: Option<serde_json::Value>,
}

/// Response describing a registered MCP server — **never** includes the token.
#[derive(Debug, Serialize)]
struct McpServerResponse {
    /// The registration's unique identifier.
    id: Uuid,
    /// The owning tenant.
    tenant_id: Uuid,
    /// The tenant-unique logical name.
    name: String,
    /// The MCP server endpoint URL.
    url: String,
    /// Whether an authorization token is stored (the token itself is never
    /// exposed).
    has_authorization: bool,
    /// The stored tool-configuration, if any.
    tool_configuration: Option<serde_json::Value>,
    /// When the registration was created.
    created_at: DateTime<Utc>,
    /// When the registration was last updated.
    updated_at: DateTime<Utc>,
}

impl From<loom_store::McpServer> for McpServerResponse {
    fn from(server: loom_store::McpServer) -> Self {
        Self {
            id: server.id,
            tenant_id: server.tenant_id,
            name: server.name,
            url: server.url,
            has_authorization: server.encrypted_token.is_some(),
            tool_configuration: server.tool_configuration,
            created_at: server.created_at,
            updated_at: server.updated_at,
        }
    }
}

/// `PUT /admin/tenants/{id}/mcp-servers/{name}` — register (or replace) a named
/// MCP server for a tenant. The authorization token is encrypted at rest and
/// bound to this `(tenant, name)` row; it is never returned in any response.
async fn put_mcp_server(
    State(state): State<AppState>,
    Path((tenant_id, name)): Path<(Uuid, String)>,
    extract::Json(req): extract::Json<PutMcpServerRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::bad_request("name must not be empty"));
    }
    if req.url.trim().is_empty() {
        return Err(ApiError::bad_request("url must not be empty"));
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

    // Encrypt the token (if any), binding the ciphertext to this row's identity
    // so it cannot be relocated into another tenant's or name's row.
    let (encrypted_token, nonce) = match req.authorization_token.as_deref() {
        Some(token) if !token.is_empty() => {
            let aad = mcp_aad(tenant_id, &name);
            let sealed = state.crypto().encrypt(token.as_bytes(), aad.as_bytes())?;
            (Some(sealed.ciphertext), Some(sealed.nonce))
        }
        _ => (None, None),
    };

    let stored = state
        .store()
        .upsert_mcp_server(NewMcpServer {
            tenant_id,
            name,
            url: req.url,
            encrypted_token,
            nonce,
            tool_configuration: req.tool_configuration,
        })
        .await
        .map_err(ApiError::from_store)?;

    Ok(Json(McpServerResponse::from(stored)))
}

/// `GET /admin/tenants/{id}/mcp-servers` — list a tenant's registered MCP
/// servers. Tokens are never included.
async fn list_mcp_servers(
    State(state): State<AppState>,
    Path(tenant_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let servers = state
        .store()
        .list_mcp_servers(tenant_id)
        .await
        .map_err(ApiError::from_store)?;
    Ok(Json(
        servers
            .into_iter()
            .map(McpServerResponse::from)
            .collect::<Vec<_>>(),
    ))
}

/// `DELETE /admin/tenants/{id}/mcp-servers/{name}` — remove a tenant's MCP
/// server registration.
async fn delete_mcp_server(
    State(state): State<AppState>,
    Path((tenant_id, name)): Path<(Uuid, String)>,
) -> Result<impl IntoResponse, ApiError> {
    if state
        .store()
        .delete_mcp_server(tenant_id, &name)
        .await
        .map_err(ApiError::from_store)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("mcp server not found"))
    }
}

/// Request body for setting a budget (tenant or key level).
#[derive(Debug, Deserialize)]
struct SetBudgetRequest {
    /// The spend limit, in the gateway's accounting currency.
    limit_amount: Decimal,
    /// The rolling window: `daily`, `weekly`, `monthly`, or `total`.
    window: String,
    /// The action on breach: `block` or `warn`.
    action: String,
}

/// `PUT /admin/tenants/{id}/budget` — set (or replace) a tenant's default
/// budget. A key-level budget overrides this at enforcement time.
async fn put_tenant_budget(
    State(state): State<AppState>,
    Path(tenant_id): Path<Uuid>,
    extract::Json(req): extract::Json<SetBudgetRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let budget = parse_budget(req.limit_amount, &req.window, &req.action)?;
    let updated = state
        .store()
        .set_tenant_budget(tenant_id, Some(budget))
        .await
        .map_err(ApiError::from_store)?;
    if updated {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("tenant not found"))
    }
}

/// `DELETE /admin/tenants/{id}/budget` — clear a tenant's default budget.
async fn delete_tenant_budget(
    State(state): State<AppState>,
    Path(tenant_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let updated = state
        .store()
        .set_tenant_budget(tenant_id, None)
        .await
        .map_err(ApiError::from_store)?;
    if updated {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("tenant not found"))
    }
}

/// `PUT /admin/keys/{id}/budget` — set (or replace) a key's budget override.
async fn put_key_budget(
    State(state): State<AppState>,
    Path(key_id): Path<Uuid>,
    extract::Json(req): extract::Json<SetBudgetRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let budget = parse_budget(req.limit_amount, &req.window, &req.action)?;
    let updated = state
        .store()
        .set_key_budget(key_id, Some(budget))
        .await
        .map_err(ApiError::from_store)?;
    if updated {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("key not found"))
    }
}

/// `DELETE /admin/keys/{id}/budget` — clear a key's budget override (the tenant
/// default, if any, then applies).
async fn delete_key_budget(
    State(state): State<AppState>,
    Path(key_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let updated = state
        .store()
        .set_key_budget(key_id, None)
        .await
        .map_err(ApiError::from_store)?;
    if updated {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("key not found"))
    }
}

/// Request body for setting a key's rate limit. Either dimension may be omitted
/// (unlimited on that dimension).
#[derive(Debug, Deserialize)]
struct SetRateLimitRequest {
    /// Maximum requests per minute, or `null`/absent for unlimited.
    #[serde(default)]
    requests_per_min: Option<i64>,
    /// Maximum tokens per minute, or `null`/absent for unlimited.
    #[serde(default)]
    tokens_per_min: Option<i64>,
}

/// `PUT /admin/keys/{id}/rate-limit` — set (or replace) a key's rate limit.
async fn put_key_rate_limit(
    State(state): State<AppState>,
    Path(key_id): Path<Uuid>,
    extract::Json(req): extract::Json<SetRateLimitRequest>,
) -> Result<impl IntoResponse, ApiError> {
    for (label, value) in [
        ("requests_per_min", req.requests_per_min),
        ("tokens_per_min", req.tokens_per_min),
    ] {
        if value.is_some_and(|v| v < 0) {
            return Err(ApiError::bad_request(format!(
                "{label} must not be negative"
            )));
        }
    }
    let rate_limit = RateLimit {
        requests_per_min: req.requests_per_min,
        tokens_per_min: req.tokens_per_min,
    };
    // An all-unlimited body is stored as "no limit" rather than a hollow row.
    let value = rate_limit.is_some().then_some(rate_limit);
    let updated = state
        .store()
        .set_key_rate_limit(key_id, value)
        .await
        .map_err(ApiError::from_store)?;
    if updated {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("key not found"))
    }
}

/// `DELETE /admin/keys/{id}/rate-limit` — clear a key's rate limit.
async fn delete_key_rate_limit(
    State(state): State<AppState>,
    Path(key_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let updated = state
        .store()
        .set_key_rate_limit(key_id, None)
        .await
        .map_err(ApiError::from_store)?;
    if updated {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("key not found"))
    }
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
