//! `GET /v1/whoami`.

use axum::Json;
use serde::Serialize;
use uuid::Uuid;

use crate::auth::TenantContext;

/// The `/v1/whoami` response: the authenticated identity.
#[derive(Serialize)]
pub(super) struct WhoAmI {
    tenant_id: Uuid,
    key_id: Uuid,
    key_prefix: String,
    scopes: Vec<String>,
}

/// `GET /v1/whoami` — echoes the resolved [`TenantContext`]. Useful for smoke
/// tests of the auth layer.
#[utoipa::path(
    get,
    path = "/v1/whoami",
    tag = "conversations",
    responses(
        (status = 200, description = "The authenticated tenant identity", body = Object),
        (status = 401, description = "Missing or invalid virtual key", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(super) async fn whoami(ctx: TenantContext) -> Json<WhoAmI> {
    Json(WhoAmI {
        tenant_id: ctx.tenant_id,
        key_id: ctx.key_id,
        key_prefix: ctx.key_prefix,
        scopes: ctx.scopes,
    })
}
