//! Cross-replica conversation serialization. The advisory transaction lock is
//! held until the response body is consumed/dropped; SQLx rolls it back on Drop.
use crate::{error::ApiError, state::AppState};
use axum::{body::Body, response::Response};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) async fn acquire(
    state: &AppState,
    tenant: Uuid,
    id: Uuid,
) -> Result<Transaction<'static, Postgres>, ApiError> {
    let hash = Sha256::digest([tenant.as_bytes().as_slice(), id.as_bytes().as_slice()].concat());
    let key = i64::from_be_bytes(hash[..8].try_into().map_err(|_| ApiError::internal())?);
    let mut tx = state
        .store()
        .pool()
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(key)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| ApiError::internal())?;
    if !locked {
        return Err(ApiError::conflict(
            "a turn is already running in this conversation",
        ));
    }
    Ok(tx)
}

pub(super) fn hold(response: Response, guard: Transaction<'static, Postgres>) -> Response {
    let (parts, body) = response.into_parts();
    let stream = futures::stream::unfold(
        (body.into_data_stream(), Some(guard)),
        |(mut body, mut guard)| async move {
            if let Some(chunk) = body.next().await {
                Some((chunk, (body, guard)))
            } else {
                if let Some(guard) = guard.take() {
                    let _ = guard.rollback().await;
                }
                None
            }
        },
    );
    Response::from_parts(parts, Body::from_stream(stream))
}
