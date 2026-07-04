//! `GET /v1/mcp-servers` — tenant-scoped MCP server name enumeration.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use loom_store::McpServerStore;

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::state::AppState;

/// The `/v1/mcp-servers` response: the tenant's registered MCP server names.
///
/// Only names are ever included — never the URL or authorization token, both
/// of which stay server-side so a virtual key can discover which
/// [`withMcp`](loom_core::McpServerRef) references are valid without learning
/// anything it could use to reach the server directly.
#[derive(Serialize, ToSchema)]
pub(super) struct McpServerList {
    /// The tenant's registered MCP server names, in store order (by name).
    servers: Vec<String>,
}

/// `GET /v1/mcp-servers` — lists the names of MCP servers registered for the
/// caller's tenant. Never exposes the URL or authorization token of a
/// registration; those remain internal to the gateway's MCP resolver.
#[utoipa::path(
    get,
    path = "/v1/mcp-servers",
    tag = "conversations",
    responses(
        (status = 200, description = "The tenant's registered MCP server names", body = McpServerList),
        (status = 401, description = "Missing or invalid virtual key", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(super) async fn list_mcp_servers(
    State(state): State<AppState>,
    ctx: TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let servers = state
        .store()
        .list_mcp_servers(ctx.tenant_id)
        .await
        .map_err(ApiError::from_store)?
        .into_iter()
        .map(|server| server.name)
        .collect();
    Ok(Json(McpServerList { servers }))
}
