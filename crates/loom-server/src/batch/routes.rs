//! The `/v1/batches` HTTP API: the sub-router, and the fetch/results/cancel
//! handlers. `create_batch` (the largest handler) lives in [`super::create`].

use axum::body::Body;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream;
use serde_json::json;
use uuid::Uuid;

use loom_store::BatchStore;

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::state::AppState;

use super::create::create_batch;
use super::dto::BatchJobDto;

/// Builds the `/v1/batches` sub-router (tenant auth is applied by the parent).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/batches", post(create_batch))
        .route("/v1/batches/{id}", get(get_batch))
        .route("/v1/batches/{id}/results", get(get_batch_results))
        .route("/v1/batches/{id}/cancel", post(cancel_batch))
}

/// `GET /v1/batches/{id}` — the batch's status and counts.
#[utoipa::path(
    get,
    path = "/v1/batches/{id}",
    tag = "batches",
    params(("id" = Uuid, Path, description = "Batch id")),
    responses(
        (status = 200, description = "The batch status and counts", body = Object),
        (status = 404, description = "No such batch for this tenant", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(crate) async fn get_batch(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let job = state
        .store()
        .get_batch_job(ctx.tenant_id, id)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::not_found("batch not found"))?;
    Ok(Json(BatchJobDto::from(job)))
}

/// `GET /v1/batches/{id}/results` — the per-item results as streamed JSONL
/// (one `{custom_id, status, result}` object per line).
#[utoipa::path(
    get,
    path = "/v1/batches/{id}/results",
    tag = "batches",
    params(("id" = Uuid, Path, description = "Batch id")),
    responses(
        (status = 200, description = "JSONL stream, one result object per line", body = String),
        (status = 404, description = "No such batch for this tenant", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(crate) async fn get_batch_results(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    // Confirm the tenant owns the batch (a missing/foreign batch is a 404).
    let job = state
        .store()
        .get_batch_job(ctx.tenant_id, id)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::not_found("batch not found"))?;

    let items = state
        .store()
        .list_batch_items(ctx.tenant_id, job.id)
        .await
        .map_err(ApiError::from_store)?;

    // One newline-terminated JSON object per item. Built up front (the item set
    // is bounded by the batch the caller submitted) and delivered as a stream.
    let lines: Vec<Result<String, std::convert::Infallible>> = items
        .into_iter()
        .map(|item| {
            let line = json!({
                "custom_id": item.custom_id,
                "status": item.status.as_str(),
                "result": item.result,
            });
            Ok(format!("{line}\n"))
        })
        .collect();

    let body = Body::from_stream(stream::iter(lines));
    let mut response = body.into_response();
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/x-ndjson"),
    );
    Ok(response)
}

/// `POST /v1/batches/{id}/cancel` — request cancellation of a batch.
#[utoipa::path(
    post,
    path = "/v1/batches/{id}/cancel",
    tag = "batches",
    params(("id" = Uuid, Path, description = "Batch id")),
    responses(
        (status = 200, description = "The batch, transitioned toward cancellation", body = Object),
        (status = 404, description = "No such batch for this tenant", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(crate) async fn cancel_batch(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let job = state
        .store()
        .request_batch_cancel(ctx.tenant_id, id)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::not_found("batch not found"))?;
    Ok(Json(BatchJobDto::from(job)))
}
