//! Tenant MCP server registry: `GET`/`PUT`/`DELETE
//! /admin/tenants/{id}/mcp-servers[/{name}]`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loom_store::{McpServerStore, NewMcpServer, TenantStore};

use crate::error::ApiError;
use crate::extract;
use crate::provider::mcp_aad;
use crate::state::AppState;

/// Request body for registering (or replacing) a tenant MCP server.
#[derive(Debug, Deserialize)]
pub(super) struct PutMcpServerRequest {
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
pub(super) async fn put_mcp_server(
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
pub(super) async fn list_mcp_servers(
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
pub(super) async fn delete_mcp_server(
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
