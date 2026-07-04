//! Tenant provisioning: `POST /admin/tenants`, `GET /admin/tenants/{id}`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use loom_store::{NewTenant, TenantStore};

use crate::error::ApiError;
use crate::extract;
use crate::state::AppState;

/// Request body for creating a tenant.
#[derive(Debug, Deserialize)]
pub(super) struct CreateTenantRequest {
    /// A stable, URL-safe unique handle for the tenant.
    slug: String,
    /// A human-readable display name.
    name: String,
}

/// `POST /admin/tenants` — create a tenant.
pub(super) async fn create_tenant(
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
pub(super) async fn get_tenant(
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
