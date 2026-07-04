//! `GET /v1/usage` — tenant-scoped usage rollups.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use loom_store::{RollupGroup, UsageRollupRow, UsageStore};

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::state::AppState;

/// Query parameters for a usage rollup.
#[derive(Debug, Deserialize)]
pub(super) struct UsageQuery {
    /// Inclusive lower bound on event time (RFC 3339); open if omitted.
    from: Option<DateTime<Utc>>,
    /// Inclusive upper bound on event time (RFC 3339); open if omitted.
    to: Option<DateTime<Utc>>,
    /// Grouping dimension: `key`, `model`, or `conversation` (default `model`).
    group_by: Option<String>,
}

/// One grouped row in a usage-rollup response.
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct UsageRollupRowDto {
    /// The group key (virtual key id, model, conversation id, or tenant id), or
    /// `null` where the grouped column was itself null.
    group: Option<String>,
    /// Number of events in the group.
    event_count: i64,
    /// Total input tokens.
    input_tokens: i64,
    /// Total output tokens.
    output_tokens: i64,
    /// Total cache-read tokens.
    cache_read_tokens: i64,
    /// Total cache-write tokens.
    cache_write_tokens: i64,
    /// Total computed cost across the group.
    cost: Decimal,
    /// The portion of `cost` from batch-tier (asynchronous) usage.
    batch_cost: Decimal,
    /// The portion of `cost` from interactive usage.
    interactive_cost: Decimal,
}

impl From<UsageRollupRow> for UsageRollupRowDto {
    fn from(row: UsageRollupRow) -> Self {
        Self {
            group: row.group,
            event_count: row.event_count,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cache_read_tokens: row.cache_read_tokens,
            cache_write_tokens: row.cache_write_tokens,
            cost: row.cost,
            batch_cost: row.batch_cost,
            interactive_cost: row.interactive_cost,
        }
    }
}

/// A usage-rollup response envelope.
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct UsageRollupResponse {
    /// The grouping dimension the rows are keyed by.
    group_by: &'static str,
    /// The lower time bound applied, if any.
    from: Option<DateTime<Utc>>,
    /// The upper time bound applied, if any.
    to: Option<DateTime<Utc>>,
    /// The grouped rows.
    rows: Vec<UsageRollupRowDto>,
}

/// Parses a tenant-scoped `group_by` value, defaulting to `model`.
fn parse_tenant_group(value: Option<&str>) -> Result<RollupGroup, ApiError> {
    match value.unwrap_or("model") {
        "key" => Ok(RollupGroup::Key),
        "model" => Ok(RollupGroup::Model),
        "conversation" => Ok(RollupGroup::Conversation),
        other => Err(ApiError::bad_request(format!(
            "unknown group_by {other:?}; expected one of key, model, conversation"
        ))),
    }
}

/// The stable label for a [`RollupGroup`].
fn group_label(group: RollupGroup) -> &'static str {
    match group {
        RollupGroup::Key => "key",
        RollupGroup::Model => "model",
        RollupGroup::Conversation => "conversation",
        RollupGroup::Tenant => "tenant",
    }
}

/// `GET /v1/usage` — tenant-scoped usage rollups grouped by key, model, or
/// conversation, over an optional `[from, to]` time window.
#[utoipa::path(
    get,
    path = "/v1/usage",
    tag = "usage",
    params(
        ("from" = Option<String>, Query, description = "Inclusive lower bound (RFC 3339)"),
        ("to" = Option<String>, Query, description = "Inclusive upper bound (RFC 3339)"),
        ("group_by" = Option<String>, Query, description = "key | model | conversation (default model)"),
    ),
    responses(
        (status = 200, description = "Grouped token and cost rollups", body = UsageRollupResponse),
        (status = 400, description = "Invalid group_by or time bound", body = Object),
        (status = 401, description = "Missing or invalid virtual key", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(super) async fn usage_rollup(
    State(state): State<AppState>,
    ctx: TenantContext,
    Query(query): Query<UsageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let group = parse_tenant_group(query.group_by.as_deref())?;
    let rows = state
        .store()
        .rollup_grouped(ctx.tenant_id, query.from, query.to, group)
        .await
        .map_err(ApiError::from_store)?;
    Ok(Json(UsageRollupResponse {
        group_by: group_label(group),
        from: query.from,
        to: query.to,
        rows: rows.into_iter().map(Into::into).collect(),
    }))
}
