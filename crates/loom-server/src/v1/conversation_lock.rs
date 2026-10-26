//! Cross-replica conversation serialization. The advisory transaction lock is
//! held until the response body is consumed/dropped. Admission reserves database
//! work capacity before borrowing a lock connection, including during cleanup.
use crate::{error::ApiError, state::AppState};
use axum::{body::Body, response::Response};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Postgres, Transaction};
use std::ops::{Deref, DerefMut};
use tokio::sync::OwnedSemaphorePermit;
use uuid::Uuid;

pub(super) struct Guard {
    transaction: Option<Transaction<'static, Postgres>>,
    admission: Option<OwnedSemaphorePermit>,
}
impl Guard {
    fn cleanup(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        let transaction = self.transaction.take()?;
        let admission = self.admission.take();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            Some(runtime.spawn(async move {
                let _ = transaction.rollback().await;
                // Retain the slot until rollback has released the connection.
                // Dropping a body or canceling its cleanup wait cannot admit
                // another lock while this old transaction is still held.
                drop(admission);
            }))
        } else {
            drop(transaction);
            drop(admission);
            None
        }
    }
    async fn release(mut self) {
        if let Some(cleanup) = self.cleanup() {
            let _ = cleanup.await;
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
impl Deref for Guard {
    type Target = PgConnection;
    fn deref(&self) -> &Self::Target {
        self.transaction
            .as_deref()
            .expect("live conversation transaction")
    }
}
impl DerefMut for Guard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.transaction
            .as_deref_mut()
            .expect("live conversation transaction")
    }
}

pub(super) async fn acquire(state: &AppState, tenant: Uuid, id: Uuid) -> Result<Guard, ApiError> {
    let admission = state.admit_conversation()?;
    let hash = Sha256::digest([tenant.as_bytes().as_slice(), id.as_bytes().as_slice()].concat());
    let key = i64::from_be_bytes(hash[..8].try_into().map_err(|_| ApiError::internal())?);
    let tx = state
        .store()
        .pool()
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
    let mut guard = Guard {
        transaction: Some(tx),
        admission: Some(admission),
    };
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(key)
        .fetch_one(&mut *guard)
        .await
        .map_err(|_| ApiError::internal())?;
    if !locked {
        return Err(ApiError::conflict(
            "a turn is already running in this conversation",
        ));
    }
    Ok(guard)
}

pub(super) fn hold(response: Response, guard: Guard) -> Response {
    let (parts, body) = response.into_parts();
    let stream = futures::stream::unfold(
        (body.into_data_stream(), Some(guard)),
        |(mut body, mut guard)| async move {
            if let Some(chunk) = body.next().await {
                Some((chunk, (body, guard)))
            } else {
                if let Some(guard) = guard.take() {
                    guard.release().await;
                }
                None
            }
        },
    );
    Response::from_parts(parts, Body::from_stream(stream))
}
