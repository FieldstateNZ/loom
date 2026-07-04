//! Rate-limit administration: `PUT`/`DELETE /admin/keys/{id}/rate-limit`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use loom_store::{BudgetStore, RateLimit};

use crate::error::ApiError;
use crate::extract;
use crate::state::AppState;

/// Request body for setting a key's rate limit. Either dimension may be omitted
/// (unlimited on that dimension).
#[derive(Debug, Deserialize)]
pub(super) struct SetRateLimitRequest {
    /// Maximum requests per minute, or `null`/absent for unlimited.
    #[serde(default)]
    requests_per_min: Option<i64>,
    /// Maximum tokens per minute, or `null`/absent for unlimited.
    #[serde(default)]
    tokens_per_min: Option<i64>,
}

/// `PUT /admin/keys/{id}/rate-limit` — set (or replace) a key's rate limit.
pub(super) async fn put_key_rate_limit(
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
pub(super) async fn delete_key_rate_limit(
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
