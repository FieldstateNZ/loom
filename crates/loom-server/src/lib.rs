//! `loom-server` — the Loom HTTP gateway.
//!
//! This crate is split into a library (the router, state, auth and admin API)
//! and a thin binary ([`main`](../main/index.html)) that loads [`Config`] from
//! the environment, connects the store, optionally runs migrations, and serves
//! [`build_router`].
//!
//! # Layout
//!
//! - [`config`] — environment-driven [`Config`], validated eagerly at boot.
//! - [`state`] — the [`AppState`] shared with every handler.
//! - [`crypto`] — AES-256-GCM envelope encryption for credentials at rest.
//! - [`keys`] — virtual-key generation and peppered-HMAC hashing.
//! - [`auth`] — tenant and admin authentication middleware and the
//!   [`TenantContext`].
//! - [`admin`] — the root-token-guarded provisioning API.
//! - [`budget`] — before-the-call budget enforcement and the spend cache.
//! - [`rate_limit`] — the in-process per-key token-bucket rate limiter.
//! - [`error`] — the structured `{ "error": { code, message, provider_error? } }`
//!   envelope.
//!
//! # Endpoints
//!
//! - `GET /healthz` — liveness (always `200`).
//! - `GET /readyz` — readiness (DB ping); `503` when the database is
//!   unreachable.
//! - `GET /v1/whoami` — echoes the authenticated [`TenantContext`] (tenant
//!   auth).
//! - `GET /v1/conversations/{id}` — a tenant-scoped resource proving isolation
//!   (tenant auth).
//! - `POST /admin/tenants`, `GET /admin/tenants/{id}`,
//!   `POST /admin/tenants/{id}/keys`, `DELETE /admin/keys/{id}`,
//!   `PUT /admin/tenants/{id}/credentials/{provider}` — admin auth.
//! - `PUT`/`DELETE /admin/tenants/{id}/budget`, `PUT`/`DELETE
//!   /admin/keys/{id}/budget`, `PUT`/`DELETE /admin/keys/{id}/rate-limit` —
//!   budget and rate-limit administration (admin auth).
//! - `GET /admin/tenants/{id}/mcp-servers`,
//!   `PUT`/`DELETE /admin/tenants/{id}/mcp-servers/{name}` — the per-tenant MCP
//!   server registry; conversations reference these by name and Loom injects
//!   the decrypted auth token server-side (admin auth).
//!
//! Budgets and per-key rate limits are enforced before the provider call on
//! both turn paths; a blocked budget returns `402 budget_exceeded` and a tripped
//! rate limit returns `429` with `Retry-After`. Enforcement state (rate-limit
//! buckets, spend cache) is in-process and therefore per-replica; see the
//! README's "Known limitation" note.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod admin;
pub mod auth;
pub mod budget;
pub mod config;
pub mod crypto;
pub mod error;
pub mod extract;
pub mod keys;
pub mod provider;
pub mod rate_limit;
pub mod state;
pub mod usage;
pub mod v1;

use axum::extract::State;
use axum::middleware::from_fn_with_state;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use http::StatusCode;
use serde::Serialize;

pub use crate::auth::TenantContext;
pub use crate::config::{Config, ConfigError};
pub use crate::crypto::{Crypto, CryptoError, EncryptedSecret};
pub use crate::error::ApiError;
pub use crate::keys::{generate_key, GeneratedKey, KeyEnv, KeyHasher};
pub use crate::provider::{DefaultProviderFactory, ProviderFactory};
pub use crate::state::AppState;
pub use crate::usage::{OutboxUsageRecorder, UsageRecorder};
pub use crate::v1::ApiDoc;

use crate::auth::{admin_auth, tenant_auth};

/// Builds the complete application router with all layers applied.
///
/// The returned router is `Router<()>` — ready to hand to `axum::serve` or to
/// drive directly in tests via `tower::ServiceExt::oneshot`.
///
/// Wires the open health/readiness/OpenAPI routes, the virtual-key-guarded
/// `/v1` conversation API, and the root-token-guarded `/admin` API, under a
/// request-tracing layer.
pub fn build_router(state: AppState) -> Router {
    let protected = v1::router().route_layer(from_fn_with_state(state.clone(), tenant_auth));

    let admin = admin::router().route_layer(from_fn_with_state(state.clone(), admin_auth));

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/openapi.json", get(v1::openapi_json))
        .merge(protected)
        .merge(admin)
        .with_state(state)
        .layer(tower_http::trace::TraceLayer::new_for_http())
}

/// Liveness endpoint. Always `200 ok`.
async fn healthz() -> &'static str {
    "ok"
}

/// The `/readyz` response body.
#[derive(Serialize)]
struct Readiness {
    status: &'static str,
}

/// Readiness endpoint: pings the database. `200` when reachable, `503`
/// otherwise (with the structured error envelope).
async fn readyz(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    sqlx::query("SELECT 1")
        .execute(state.store().pool())
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "readiness probe failed");
            ApiError::unavailable("database is not reachable")
        })?;
    Ok((StatusCode::OK, Json(Readiness { status: "ready" })))
}
