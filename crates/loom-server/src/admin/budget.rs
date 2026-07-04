//! Budget administration: `PUT`/`DELETE /admin/tenants/{id}/budget` and
//! `PUT`/`DELETE /admin/keys/{id}/budget`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use http::StatusCode;
use rust_decimal::Decimal;
use serde::Deserialize;
use uuid::Uuid;

use loom_store::BudgetStore;

use crate::budget::parse_budget;
use crate::error::ApiError;
use crate::extract;
use crate::state::AppState;

/// Request body for setting a budget (tenant or key level).
#[derive(Debug, Deserialize)]
pub(super) struct SetBudgetRequest {
    /// The spend limit, in the gateway's accounting currency.
    limit_amount: Decimal,
    /// The rolling window: `daily`, `weekly`, `monthly`, or `total`.
    window: String,
    /// The action on breach: `block` or `warn`.
    action: String,
}

/// `PUT /admin/tenants/{id}/budget` — set (or replace) a tenant's default
/// budget. A key-level budget overrides this at enforcement time.
pub(super) async fn put_tenant_budget(
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
pub(super) async fn delete_tenant_budget(
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
pub(super) async fn put_key_budget(
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
pub(super) async fn delete_key_budget(
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
