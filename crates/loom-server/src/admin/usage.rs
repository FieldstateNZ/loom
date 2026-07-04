//! Gateway-wide usage reporting: `GET /admin/usage`.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::Serialize;
use utoipa::ToSchema;

use loom_store::UsageStore;

use crate::error::ApiError;
use crate::state::AppState;

/// Query parameters for the gateway-wide usage rollup.
#[derive(Debug, Deserialize)]
pub(crate) struct AdminUsageQuery {
    /// Inclusive lower bound on event time (RFC 3339); open if omitted.
    from: Option<DateTime<Utc>>,
    /// Inclusive upper bound on event time (RFC 3339); open if omitted.
    to: Option<DateTime<Utc>>,
    /// Grouping dimension; only `tenant` is supported gateway-wide.
    group_by: Option<String>,
}

/// One tenant's aggregate usage in the gateway-wide rollup.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AdminUsageRow {
    /// The tenant id.
    group: Option<String>,
    event_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    cost: Decimal,
}

/// The gateway-wide usage response envelope.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AdminUsageResponse {
    group_by: &'static str,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    rows: Vec<AdminUsageRow>,
}

/// `GET /admin/usage?group_by=tenant` — gateway-wide usage rolled up by tenant,
/// over an optional `[from, to]` window. Root-token only.
#[utoipa::path(
    get,
    path = "/admin/usage",
    tag = "admin",
    params(
        ("from" = Option<String>, Query, description = "Inclusive lower bound (RFC 3339)"),
        ("to" = Option<String>, Query, description = "Inclusive upper bound (RFC 3339)"),
        ("group_by" = Option<String>, Query, description = "Only 'tenant' is supported gateway-wide"),
    ),
    responses(
        (status = 200, description = "Gateway-wide usage rolled up by tenant", body = AdminUsageResponse),
        (status = 400, description = "Unsupported group_by", body = Object),
        (status = 401, description = "Missing or invalid admin token", body = Object),
    ),
    security(("admin_token" = []))
)]
pub(crate) async fn usage_by_tenant(
    State(state): State<AppState>,
    Query(query): Query<AdminUsageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    // Gateway-wide reporting groups by tenant; reject any other dimension.
    if let Some(group_by) = query.group_by.as_deref() {
        if group_by != "tenant" {
            return Err(ApiError::bad_request(format!(
                "unknown group_by {group_by:?}; only 'tenant' is supported here"
            )));
        }
    }
    let rows = state
        .store()
        .rollup_by_tenant(query.from, query.to)
        .await
        .map_err(ApiError::from_store)?;
    Ok(Json(AdminUsageResponse {
        group_by: "tenant",
        from: query.from,
        to: query.to,
        rows: rows
            .into_iter()
            .map(|r| AdminUsageRow {
                group: r.group,
                event_count: r.event_count,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cache_read_tokens: r.cache_read_tokens,
                cache_write_tokens: r.cache_write_tokens,
                cost: r.cost,
            })
            .collect(),
    }))
}
