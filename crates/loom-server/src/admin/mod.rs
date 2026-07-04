//! The `/admin` API: tenant, virtual-key and credential provisioning, plus
//! budget and rate-limit administration.
//!
//! Every route here is guarded by [`admin_auth`](crate::auth::admin_auth), so
//! handlers assume the caller holds the root admin token. Responses use the
//! shared [`ApiError`](crate::error::ApiError) envelope on failure.
//!
//! # Layout
//!
//! - [`tenant`] — tenant create/fetch.
//! - [`key`] — virtual-key mint/revoke.
//! - [`credential`] — provider credential storage.
//! - [`mcp`] — the per-tenant MCP server registry.
//! - [`budget`] — tenant/key budget administration.
//! - [`rate_limit`] — key rate-limit administration.
//! - [`usage`] — the gateway-wide usage rollup.

mod budget;
mod credential;
mod key;
mod mcp;
mod rate_limit;
mod tenant;
mod usage;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::state::AppState;

/// Builds the `/admin` sub-router (without its guard layer, which the top-level
/// router applies).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/tenants", post(tenant::create_tenant))
        .route("/admin/tenants/{id}", get(tenant::get_tenant))
        .route("/admin/tenants/{id}/keys", post(key::create_key))
        .route("/admin/keys/{id}", delete(key::revoke_key))
        .route(
            "/admin/tenants/{id}/credentials/{provider}",
            put(credential::put_credential),
        )
        .route(
            "/admin/tenants/{id}/mcp-servers",
            get(mcp::list_mcp_servers),
        )
        .route(
            "/admin/tenants/{id}/mcp-servers/{name}",
            put(mcp::put_mcp_server).delete(mcp::delete_mcp_server),
        )
        .route(
            "/admin/tenants/{id}/budget",
            put(budget::put_tenant_budget).delete(budget::delete_tenant_budget),
        )
        .route(
            "/admin/keys/{id}/budget",
            put(budget::put_key_budget).delete(budget::delete_key_budget),
        )
        .route(
            "/admin/keys/{id}/rate-limit",
            put(rate_limit::put_key_rate_limit).delete(rate_limit::delete_key_rate_limit),
        )
        .route("/admin/usage", get(usage::usage_by_tenant))
}
