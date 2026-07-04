//! Authentication middleware and the per-request tenant context.
//!
//! Two independent layers guard the gateway:
//!
//! - [`tenant_auth`] protects tenant-facing routes. It resolves a
//!   `Authorization: Bearer loom_...` virtual key to a [`TenantContext`] and
//!   inserts it into the request extensions for downstream handlers.
//! - [`admin_auth`] protects the `/admin` API with the constant-time-compared
//!   root admin token.
//!
//! # Revocation
//!
//! There is no in-process cache of keys: every request performs a fresh
//! `KeyStore::get_key_by_hash` lookup and checks the key's status, so revoking a
//! key takes effect immediately on the next request. (A short-TTL cache with an
//! explicit bust could be layered on later if the lookup ever becomes hot.)

use axum::extract::{FromRequestParts, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use http::request::Parts;
use uuid::Uuid;

use loom_store::{KeyStore, TenantStore};

use crate::error::ApiError;
use crate::state::AppState;

/// The identity resolved from a virtual key, attached to every authenticated
/// request.
///
/// Handlers obtain it as an extractor (`ctx: TenantContext`); it is also present
/// in the request extensions. Every tenant-scoped store call must pass
/// [`tenant_id`](Self::tenant_id) so isolation is enforced at the data layer.
#[derive(Clone, Debug)]
pub struct TenantContext {
    /// The authenticated tenant.
    pub tenant_id: Uuid,
    /// The virtual key that authenticated the request.
    pub key_id: Uuid,
    /// The key's non-secret display prefix (safe for logs).
    pub key_prefix: String,
    /// The scopes granted to the key.
    pub scopes: Vec<String>,
}

impl<S> FromRequestParts<S> for TenantContext
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<TenantContext>()
            .cloned()
            .ok_or_else(|| ApiError::unauthorized("authentication required"))
    }
}

/// Extracts the bearer token from an `Authorization` header, if present.
fn bearer_token(parts: &Parts) -> Option<&str> {
    parts
        .headers
        .get(http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// Middleware that authenticates a tenant virtual key.
///
/// On success it inserts a [`TenantContext`] into the request extensions and
/// (best effort) bumps the key's `last_used_at`. On any failure it returns a
/// `401` with the structured error envelope.
///
/// # Errors
///
/// Returns `401 Unauthorized` when the `Authorization` header is missing or
/// malformed, the key is unknown or revoked, or the owning tenant is not active.
pub async fn tenant_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let (mut parts, body) = req.into_parts();

    let token = bearer_token(&parts)
        .ok_or_else(|| ApiError::unauthorized("missing bearer token"))?
        .to_owned();
    if !token.starts_with("loom_") {
        return Err(ApiError::unauthorized("invalid api key"));
    }

    let key_hash = state.hasher().hash(&token);
    let key = state
        .store()
        .get_key_by_hash(&key_hash)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::unauthorized("invalid api key"))?;

    if key.status != "active" {
        return Err(ApiError::unauthorized("api key has been revoked"));
    }

    let tenant = state
        .store()
        .get_tenant(key.tenant_id)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::unauthorized("invalid api key"))?;
    if tenant.status != "active" {
        return Err(ApiError::unauthorized("tenant is not active"));
    }

    // Best effort: a failure to record last-used must not fail the request.
    if let Err(err) = state.store().touch_key_last_used(key.id).await {
        tracing::warn!(error = %err, key_id = %key.id, "failed to touch key last_used_at");
    }

    parts.extensions.insert(TenantContext {
        tenant_id: key.tenant_id,
        key_id: key.id,
        key_prefix: key.key_prefix,
        scopes: key.scopes,
    });

    Ok(next.run(Request::from_parts(parts, body)).await)
}

/// Middleware that guards the `/admin` API with the root admin token.
///
/// The presented bearer token is compared to the configured root token in
/// constant time (see [`AppState::verify_admin_token`]).
///
/// # Errors
///
/// Returns `401 Unauthorized` when the token is missing or does not match.
pub async fn admin_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let (parts, body) = req.into_parts();
    let token =
        bearer_token(&parts).ok_or_else(|| ApiError::unauthorized("admin token required"))?;
    if !state.verify_admin_token(token) {
        return Err(ApiError::unauthorized("invalid admin token"));
    }
    Ok(next.run(Request::from_parts(parts, body)).await)
}
