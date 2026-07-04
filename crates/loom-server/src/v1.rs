//! The `/v1` conversation API: tenant-scoped conversations and turns.
//!
//! Every route here is guarded by [`tenant_auth`](crate::auth::tenant_auth), so
//! handlers receive a resolved [`TenantContext`] and scope every store call to
//! it. The turn endpoints resolve the bound [`Provider`] through the
//! [`AppState`]'s provider factory, run capability negotiation, and either
//! return the assistant [`Message`] (non-streaming) or an SSE stream of
//! [`TurnEvent`] envelopes (streaming). The stateful and stateless turn paths
//! share one core runner ([`execute_turn`]) so their behaviour cannot drift.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use futures::{stream, StreamExt};
use http::header::HeaderValue;
use http::StatusCode;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi, ToSchema};
use uuid::Uuid;

use loom_core::{
    CacheHint, Conversation, ConversationOptions, Message, ProviderBinding, Role, Usage,
};
use loom_provider::{
    ensure_supported, required_capabilities, Capability, Provider, ProviderError, TurnEvent,
    TurnEventKind, TurnEventStream,
};
use loom_store::{
    ConversationStore, NewUsageEvent, Pricer, PricingStore, RollupGroup, UsageRollupRow, UsageStore,
};

use crate::auth::TenantContext;
use crate::budget::{self, BudgetWarning};
use crate::error::ApiError;
use crate::extract;
use crate::state::AppState;
use crate::telemetry;

/// Response header set when a `warn`-action budget is over its soft limit.
const BUDGET_WARNING_HEADER: &str = "x-loom-budget-warning";

/// The default number of messages returned by a conversation fetch.
const DEFAULT_MESSAGE_LIMIT: i64 = 100;
/// The maximum number of messages returned by a single conversation fetch.
const MAX_MESSAGE_LIMIT: i64 = 1000;

/// Builds the `/v1` sub-router (without its auth layer, which the top-level
/// router applies).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/whoami", get(whoami))
        .route("/v1/conversations", post(create_conversation))
        .route(
            "/v1/conversations/{id}",
            get(get_conversation).delete(delete_conversation),
        )
        .route("/v1/conversations/{id}/turns", post(create_turn))
        .route("/v1/turns", post(stateless_turn))
        .route("/v1/usage", get(usage_rollup))
}

/// The `/v1/whoami` response: the authenticated identity.
#[derive(Serialize)]
struct WhoAmI {
    tenant_id: Uuid,
    key_id: Uuid,
    key_prefix: String,
    scopes: Vec<String>,
}

/// `GET /v1/whoami` — echoes the resolved [`TenantContext`]. Useful for smoke
/// tests of the auth layer.
#[utoipa::path(
    get,
    path = "/v1/whoami",
    tag = "conversations",
    responses(
        (status = 200, description = "The authenticated tenant identity", body = Object),
        (status = 401, description = "Missing or invalid virtual key", body = Object),
    ),
    security(("virtual_key" = []))
)]
async fn whoami(ctx: TenantContext) -> Json<WhoAmI> {
    Json(WhoAmI {
        tenant_id: ctx.tenant_id,
        key_id: ctx.key_id,
        key_prefix: ctx.key_prefix,
        scopes: ctx.scopes,
    })
}

/// Request body for creating a conversation.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateConversationRequest {
    /// The provider to bind the conversation to (e.g. `"anthropic"`).
    pub provider: String,
    /// The model identifier, as the provider expects it.
    pub model: String,
    /// An optional system prompt applied to the whole conversation.
    #[serde(default)]
    pub system: Option<String>,
    /// Free-form caller metadata (tags, correlation IDs, …).
    #[serde(default)]
    #[schema(value_type = Object, nullable)]
    pub metadata: Option<serde_json::Value>,
}

/// `POST /v1/conversations` — create a tenant-scoped conversation.
#[utoipa::path(
    post,
    path = "/v1/conversations",
    tag = "conversations",
    request_body = CreateConversationRequest,
    responses(
        (status = 201, description = "Conversation created", body = Object),
        (status = 400, description = "Malformed request", body = Object),
        (status = 401, description = "Missing or invalid virtual key", body = Object),
    ),
    security(("virtual_key" = []))
)]
async fn create_conversation(
    State(state): State<AppState>,
    ctx: TenantContext,
    extract::Json(req): extract::Json<CreateConversationRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.provider.trim().is_empty() || req.model.trim().is_empty() {
        return Err(ApiError::bad_request(
            "provider and model must not be empty",
        ));
    }

    let mut conversation =
        Conversation::new(ctx.tenant_id, ProviderBinding::new(req.provider, req.model));
    conversation.system = req.system;
    if let Some(metadata) = req.metadata {
        conversation.metadata = metadata;
    }

    state
        .store()
        .create_conversation(&conversation)
        .await
        .map_err(ApiError::from_store)?;

    Ok((StatusCode::CREATED, Json(conversation)))
}

/// Query parameters selecting a page of a conversation's message history.
#[derive(Debug, Deserialize)]
struct Pagination {
    /// Maximum number of messages to return.
    limit: Option<i64>,
    /// Number of messages to skip from the start of the history.
    offset: Option<i64>,
}

/// `GET /v1/conversations/{id}` — fetch a conversation with a page of its
/// history, scoped to the caller's tenant.
#[utoipa::path(
    get,
    path = "/v1/conversations/{id}",
    tag = "conversations",
    params(
        ("id" = Uuid, Path, description = "Conversation id"),
        ("limit" = Option<i64>, Query, description = "Max messages to return (default 100)"),
        ("offset" = Option<i64>, Query, description = "Messages to skip from the start"),
    ),
    responses(
        (status = 200, description = "The conversation and a page of its messages", body = Object),
        (status = 404, description = "No such conversation for this tenant", body = Object),
    ),
    security(("virtual_key" = []))
)]
async fn get_conversation(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(id): Path<Uuid>,
    Query(page): Query<Pagination>,
) -> Result<impl IntoResponse, ApiError> {
    let mut conversation = state
        .store()
        .get_conversation(ctx.tenant_id, id)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::not_found("conversation not found"))?;

    let limit = page
        .limit
        .unwrap_or(DEFAULT_MESSAGE_LIMIT)
        .clamp(1, MAX_MESSAGE_LIMIT);
    let offset = page.offset.unwrap_or(0).max(0);

    conversation.messages = state
        .store()
        .list_messages(ctx.tenant_id, id, limit, offset)
        .await
        .map_err(ApiError::from_store)?;

    Ok(Json(conversation))
}

/// `DELETE /v1/conversations/{id}` — delete a conversation scoped to the
/// caller's tenant.
#[utoipa::path(
    delete,
    path = "/v1/conversations/{id}",
    tag = "conversations",
    params(("id" = Uuid, Path, description = "Conversation id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such conversation for this tenant", body = Object),
    ),
    security(("virtual_key" = []))
)]
async fn delete_conversation(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    if state
        .store()
        .delete_conversation(ctx.tenant_id, id)
        .await
        .map_err(ApiError::from_store)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("conversation not found"))
    }
}

/// Request body for appending a turn to a stored conversation.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TurnRequest {
    /// The user turn's content parts, in provider-significant order.
    #[schema(value_type = Vec<Object>)]
    pub content: Vec<loom_core::ContentPart>,
    /// Whether to stream the assistant turn as Server-Sent Events.
    #[serde(default)]
    pub stream: bool,
    /// Request-time provider options (sampling, tools, …).
    #[serde(default)]
    #[schema(value_type = Object, nullable)]
    pub options: Option<ConversationOptions>,
}

/// `POST /v1/conversations/{id}/turns` — append a user turn, run the bound
/// provider, and return (or stream) the assistant turn, persisting both.
#[utoipa::path(
    post,
    path = "/v1/conversations/{id}/turns",
    tag = "conversations",
    params(("id" = Uuid, Path, description = "Conversation id")),
    request_body = TurnRequest,
    responses(
        (status = 200, description = "The assistant message, or an SSE stream when stream=true", body = Object),
        (status = 404, description = "No such conversation for this tenant", body = Object),
        (status = 422, description = "Capability unsupported or provider not configured", body = Object),
    ),
    security(("virtual_key" = []))
)]
async fn create_turn(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(id): Path<Uuid>,
    extract::Json(req): extract::Json<TurnRequest>,
) -> Result<Response, ApiError> {
    if req.content.is_empty() {
        return Err(ApiError::bad_request("content must not be empty"));
    }

    let mut conversation = state
        .store()
        .get_conversation(ctx.tenant_id, id)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::not_found("conversation not found"))?;

    let mut options = req.options.unwrap_or_default();
    let user_turn = Message::new(Role::User, req.content);

    // Resolve named MCP server references to their URL + decrypted token
    // *before* persisting the user turn, so an unconfigured server fails fast
    // without mutating history and the auth token is injected server-side.
    crate::provider::resolve_mcp_servers(&state, ctx.tenant_id, &mut options).await?;

    // Resolve the provider and negotiate capabilities *before* persisting the
    // user turn, so a doomed request (no credential, unsupported capability)
    // fails fast without mutating history.
    let provider = state
        .resolve_provider(ctx.tenant_id, &conversation.binding.provider)
        .await?;
    conversation.messages.push(user_turn.clone());
    negotiate(provider.as_ref(), &conversation, &options, req.stream)?;

    // Enforce rate limits and budget *before* the provider call (and before
    // persisting the user turn, so a blocked request leaves history untouched).
    let warning = enforce_limits(&state, &ctx).await?;

    // Persist the user turn now that the request is known to be runnable.
    state
        .store()
        .append_message(ctx.tenant_id, id, &user_turn)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::not_found("conversation not found"))?;

    execute_turn(
        provider,
        &state,
        conversation,
        options,
        req.stream,
        true,
        ctx.key_id,
        warning,
    )
    .await
}

/// Request body for a stateless turn: the whole conversation is supplied inline
/// and nothing is persisted.
#[derive(Debug, Deserialize, ToSchema)]
pub struct StatelessTurnRequest {
    /// The provider to run against (e.g. `"anthropic"`).
    pub provider: String,
    /// The model identifier, as the provider expects it.
    pub model: String,
    /// An optional system prompt.
    #[serde(default)]
    pub system: Option<String>,
    /// An optional prompt-cache breakpoint on the system prompt.
    #[serde(default)]
    #[schema(value_type = Object, nullable)]
    pub system_cache: Option<CacheHint>,
    /// The full, inline message history to run.
    #[schema(value_type = Vec<Object>)]
    pub messages: Vec<Message>,
    /// Request-time provider options.
    #[serde(default)]
    #[schema(value_type = Object, nullable)]
    pub options: Option<ConversationOptions>,
    /// Whether to stream the assistant turn as Server-Sent Events.
    #[serde(default)]
    pub stream: bool,
}

/// `POST /v1/turns` — run the provider on a fully-inline conversation with **no**
/// persistence. Shares [`execute_turn`] with the stateful path for parity.
#[utoipa::path(
    post,
    path = "/v1/turns",
    tag = "conversations",
    request_body = StatelessTurnRequest,
    responses(
        (status = 200, description = "The assistant message, or an SSE stream when stream=true", body = Object),
        (status = 400, description = "Malformed request", body = Object),
        (status = 422, description = "Capability unsupported or provider not configured", body = Object),
    ),
    security(("virtual_key" = []))
)]
async fn stateless_turn(
    State(state): State<AppState>,
    ctx: TenantContext,
    extract::Json(req): extract::Json<StatelessTurnRequest>,
) -> Result<Response, ApiError> {
    if req.provider.trim().is_empty() || req.model.trim().is_empty() {
        return Err(ApiError::bad_request(
            "provider and model must not be empty",
        ));
    }
    if req.messages.is_empty() {
        return Err(ApiError::bad_request("messages must not be empty"));
    }

    let mut options = req.options.unwrap_or_default();
    let mut conversation =
        Conversation::new(ctx.tenant_id, ProviderBinding::new(req.provider, req.model));
    conversation.system = req.system;
    conversation.system_cache = req.system_cache;
    conversation.messages = req.messages;

    // Resolve named MCP server references (URL + decrypted token injected
    // server-side) before dispatch.
    crate::provider::resolve_mcp_servers(&state, ctx.tenant_id, &mut options).await?;

    let provider = state
        .resolve_provider(ctx.tenant_id, &conversation.binding.provider)
        .await?;
    negotiate(provider.as_ref(), &conversation, &options, req.stream)?;

    // Enforce rate limits and budget before the provider call.
    let warning = enforce_limits(&state, &ctx).await?;

    execute_turn(
        provider,
        &state,
        conversation,
        options,
        req.stream,
        false,
        ctx.key_id,
        warning,
    )
    .await
}

/// Enforces per-key rate limits and the effective budget before a provider
/// call, shared by the stateful and stateless turn paths.
///
/// Returns `Err` with a `429` (rate limit) or `402` (blocked budget); returns
/// `Ok(Some(warning))` when a `warn`-action budget is over its soft limit (the
/// caller surfaces it as the `x-loom-budget-warning` header) and `Ok(None)`
/// otherwise.
async fn enforce_limits(
    state: &AppState,
    ctx: &TenantContext,
) -> Result<Option<BudgetWarning>, ApiError> {
    if let Err(rejection) = state
        .rate_limiter()
        .check(ctx.key_id, ctx.rate_limit.as_ref())
    {
        return Err(ApiError::rate_limited(
            format!("{} rate limit exceeded", rejection.kind.label()),
            rejection.retry_after_secs(),
        ));
    }
    budget::enforce(state, ctx).await
}

/// Query parameters for a usage rollup.
#[derive(Debug, Deserialize)]
struct UsageQuery {
    /// Inclusive lower bound on event time (RFC 3339); open if omitted.
    from: Option<DateTime<Utc>>,
    /// Inclusive upper bound on event time (RFC 3339); open if omitted.
    to: Option<DateTime<Utc>>,
    /// Grouping dimension: `key`, `model`, or `conversation` (default `model`).
    group_by: Option<String>,
}

/// One grouped row in a usage-rollup response.
#[derive(Debug, Serialize)]
struct UsageRollupRowDto {
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
        }
    }
}

/// A usage-rollup response envelope.
#[derive(Debug, Serialize)]
struct UsageRollupResponse {
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
        (status = 200, description = "Grouped token and cost rollups", body = Object),
        (status = 400, description = "Invalid group_by or time bound", body = Object),
        (status = 401, description = "Missing or invalid virtual key", body = Object),
    ),
    security(("virtual_key" = []))
)]
async fn usage_rollup(
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

/// Runs capability negotiation for a turn, failing fast with a structured error.
///
/// Mirrors what a provider does internally, but runs *before* any state is
/// mutated so an unsupported request never persists a partial turn.
fn negotiate(
    provider: &dyn Provider,
    conversation: &Conversation,
    options: &ConversationOptions,
    stream: bool,
) -> Result<(), ApiError> {
    let descriptor = provider.descriptor();
    let model = descriptor
        .model(&conversation.binding.model)
        .ok_or_else(|| {
            ApiError::from_provider(ProviderError::ModelNotFound {
                provider: descriptor.name.clone(),
                model: conversation.binding.model.clone(),
            })
        })?;

    let mut required = required_capabilities(conversation, options);
    if stream {
        required.insert(Capability::Streaming);
    }
    ensure_supported(&descriptor.name, model, &required).map_err(ApiError::from_provider)
}

/// The identity a turn's usage is attributed to.
///
/// `conversation_id` is `Some` only for stateful turns; a stateless turn
/// records usage against the tenant and key with no conversation.
#[derive(Clone)]
struct UsageAttribution {
    tenant_id: Uuid,
    virtual_key_id: Option<Uuid>,
    conversation_id: Option<Uuid>,
    provider: String,
    model: String,
}

/// The shared core of both turn paths: runs the provider, records a priced
/// usage event for the turn, and — when `persist` is set — records the
/// assistant turn against `conversation`.
///
/// Non-streaming returns the assistant [`Message`] as JSON; streaming returns an
/// SSE response of [`TurnEvent`] envelopes, recording usage and persisting the
/// reassembled assistant message when the stream ends.
#[allow(clippy::too_many_arguments)]
async fn execute_turn(
    provider: std::sync::Arc<dyn Provider>,
    state: &AppState,
    conversation: Conversation,
    options: ConversationOptions,
    stream: bool,
    persist: bool,
    key_id: Uuid,
    warning: Option<BudgetWarning>,
) -> Result<Response, ApiError> {
    let attribution = UsageAttribution {
        tenant_id: conversation.tenant_id,
        virtual_key_id: Some(key_id),
        // Stateless turns are not tied to a stored conversation.
        conversation_id: persist.then_some(conversation.id),
        provider: conversation.binding.provider.clone(),
        model: conversation.binding.model.clone(),
    };

    // The turn span roots the provider (and store) spans for this turn; it nests
    // under the HTTP request span the outer middleware opened. It carries the
    // tenant and model, never message content.
    let turn_span = tracing::info_span!(
        "conversation.turn",
        "loom.tenant.id" = %attribution.tenant_id,
        "gen_ai.system" = %attribution.provider,
        "gen_ai.request.model" = %attribution.model,
        "loom.stream" = stream,
    );
    let started = Instant::now();

    let mut response = if stream {
        let provider_span = telemetry::provider_span(
            &turn_span,
            &attribution.provider,
            &attribution.model,
            true,
            attribution.tenant_id,
        );
        let events = provider
            .stream(&conversation, &options)
            .instrument(provider_span.clone())
            .await
            .map_err(ApiError::from_provider)?;
        // Input content is attached only when the debug capture flag is set; the
        // span stays open across the whole stream so the output content and final
        // usage attributes are recorded when the stream settles.
        telemetry::record_input_content(&provider_span, || content_digest(&conversation));
        // The active-streams gauge is incremented/decremented by an RAII guard
        // owned by the SSE stream state (see `ActiveStreamGuard`), so it is
        // balanced on every termination path — including a mid-stream client
        // disconnect that drops the body future without a clean or error end.
        let persist_target = persist.then_some((conversation.tenant_id, conversation.id));
        sse_response(
            events,
            state.clone(),
            attribution,
            persist_target,
            provider_span,
            started,
        )
    } else {
        let provider_span = telemetry::provider_span(
            &turn_span,
            &attribution.provider,
            &attribution.model,
            false,
            attribution.tenant_id,
        );
        let message = provider
            .complete(&conversation, &options)
            .instrument(provider_span.clone())
            .await
            .map_err(ApiError::from_provider)?;
        state.metrics().record_provider_call(
            &attribution.provider,
            &attribution.model,
            false,
            started.elapsed(),
        );
        // Capture usage for every turn, before persisting the message.
        let usage = message.usage.clone().unwrap_or_default();
        let cost = record_turn_usage(state, &attribution, usage.clone()).await;
        telemetry::record_provider_result(
            &provider_span,
            &usage,
            cost,
            stop_reason_from_message(&message).as_deref(),
        );
        telemetry::record_input_content(&provider_span, || content_digest(&conversation));
        telemetry::record_output_content(&provider_span, || message_digest(&message));
        if persist {
            let store_span = tracing::info_span!(
                parent: &turn_span,
                "store.append_message",
                "loom.conversation.id" = %conversation.id,
            );
            state
                .store()
                .append_message(conversation.tenant_id, conversation.id, &message)
                .instrument(store_span)
                .await
                .map_err(ApiError::from_store)?;
        }
        Json(message).into_response()
    };

    // A `warn`-action budget over its soft limit lets the turn proceed but flags
    // it to the caller via a response header.
    if let Some(warning) = warning {
        if let Ok(value) = HeaderValue::from_str(warning.header_value()) {
            response.headers_mut().insert(BUDGET_WARNING_HEADER, value);
        }
    }
    Ok(response)
}

/// Records a priced usage event for a completed turn (best effort).
///
/// The cost is computed at write time from the effective price for
/// `(provider, model)` at the current instant; if no price is configured or the
/// lookup fails, the event is still recorded with `cost = None` so the raw usage
/// is never lost and cost can be recomputed later. The write itself goes through
/// the state's [`UsageRecorder`](crate::usage::UsageRecorder), which parks the
/// event in the outbox on failure — a usage-write fault never fails the turn.
async fn record_turn_usage(
    state: &AppState,
    attribution: &UsageAttribution,
    usage: Usage,
) -> Option<Decimal> {
    let cost = match state
        .store()
        .get_effective_price(&attribution.provider, &attribution.model, Utc::now())
        .await
    {
        Ok(Some(price)) => Some(Pricer::cost(&usage, &price)),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(error = %err, "price lookup failed; recording usage without cost");
            None
        }
    };
    // Debit the key's per-minute token bucket now that the turn's usage is known
    // (a no-op when the key has no token limit).
    if let Some(key_id) = attribution.virtual_key_id {
        let tokens = usage
            .input_tokens
            .unwrap_or(0)
            .saturating_add(usage.output_tokens.unwrap_or(0))
            .saturating_add(usage.cache_read_tokens.unwrap_or(0))
            .saturating_add(usage.cache_write_tokens.unwrap_or(0));
        state.rate_limiter().record_tokens(key_id, tokens);
    }
    // Emit token/cost metrics by tenant + model (no-ops without a meter).
    state.metrics().record_tokens(
        attribution.tenant_id,
        &attribution.model,
        usage.input_tokens.unwrap_or(0),
        usage.output_tokens.unwrap_or(0),
    );
    if let Some(cost) = cost {
        state
            .metrics()
            .record_cost(attribution.tenant_id, &attribution.model, cost);
    }
    let event = NewUsageEvent {
        tenant_id: attribution.tenant_id,
        virtual_key_id: attribution.virtual_key_id,
        conversation_id: attribution.conversation_id,
        provider: attribution.provider.clone(),
        model: attribution.model.clone(),
        usage,
        cost,
        is_batch: false,
    };
    state.usage_recorder().record(state.store(), event).await;
    cost
}

/// A best-effort stop-reason string for a completed non-streaming turn, read
/// from the provider's verbatim native response payload. Content is never read —
/// only the `stop_reason` field. `None` when absent.
fn stop_reason_from_message(message: &Message) -> Option<String> {
    message
        .raw
        .as_ref()?
        .get("stop_reason")?
        .as_str()
        .map(str::to_owned)
}

/// A compact, telemetry-only digest of a conversation's inputs (system prompt +
/// messages) as JSON. Built **only** when content capture is enabled.
fn content_digest(conversation: &Conversation) -> String {
    serde_json::json!({
        "system": conversation.system,
        "messages": conversation.messages,
    })
    .to_string()
}

/// A compact, telemetry-only digest of an assistant message as JSON. Built
/// **only** when content capture is enabled.
fn message_digest(message: &Message) -> String {
    serde_json::to_string(message).unwrap_or_default()
}

/// The stable snake_case label for a [`StopReason`], mirroring its serde
/// representation and preserving the verbatim string for `Other`.
fn stop_reason_label(reason: &loom_provider::StopReason) -> String {
    use loom_provider::StopReason;
    match reason {
        StopReason::EndTurn => "end_turn".to_owned(),
        StopReason::MaxTokens => "max_tokens".to_owned(),
        StopReason::StopSequence => "stop_sequence".to_owned(),
        StopReason::ToolUse => "tool_use".to_owned(),
        StopReason::PauseTurn => "pause_turn".to_owned(),
        StopReason::Refusal => "refusal".to_owned(),
        StopReason::Other(s) => s.clone(),
        _ => "unknown".to_owned(),
    }
}

/// An RAII guard that keeps the `loom.streams.active` gauge balanced.
///
/// Constructing one increments the gauge; dropping one decrements it. Because
/// the decrement lives in [`Drop`] it fires on *every* stream termination path —
/// a clean end, a provider error, and (critically) a mid-stream client
/// disconnect, where hyper drops the SSE body future without the unfold ever
/// reaching its terminal branch. The gauge decrement is intentionally the guard's
/// *sole* responsibility (it is not also done in [`SseState::finalize`]) so a
/// stream can never be double-counted.
struct ActiveStreamGuard {
    metrics: telemetry::Metrics,
}

impl ActiveStreamGuard {
    /// Marks a stream as open (increments the active-streams gauge). The paired
    /// decrement fires when the guard is dropped.
    fn new(metrics: telemetry::Metrics) -> Self {
        metrics.stream_started();
        Self { metrics }
    }
}

impl Drop for ActiveStreamGuard {
    fn drop(&mut self) {
        // Synchronous and safe in `Drop`: a bare gauge decrement, no `.await`.
        self.metrics.stream_ended();
    }
}

/// The mutable state driving the SSE `unfold`.
struct SseState {
    events: TurnEventStream,
    accumulator: TurnAccumulator,
    state: AppState,
    attribution: UsageAttribution,
    /// `(tenant_id, conversation_id)` for message persistence, or `None` for a
    /// stateless turn. Usage is recorded regardless.
    persist: Option<(Uuid, Uuid)>,
    /// The provider span, kept open across the whole stream so first-token
    /// latency, final usage and the stop reason are recorded on it.
    span: tracing::Span,
    /// When the turn started, for first-token latency and provider duration.
    started: Instant,
    /// Whether the first-token event has already been recorded.
    first_token_seen: bool,
    finalized: bool,
    done: bool,
    /// Balances the active-streams gauge for the lifetime of the stream. Held
    /// only for its [`Drop`] side effect (decrement on every termination path);
    /// never read directly.
    _gauge: ActiveStreamGuard,
}

/// Builds the SSE [`Response`] for a streamed turn.
///
/// Each provider [`TurnEvent`] is serialised verbatim (normalised envelope plus
/// raw native event) as one `data:` frame. When the underlying stream ends —
/// cleanly or via a provider error — a priced usage event is recorded and, for
/// a stateful turn, the reassembled assistant [`Message`] is persisted (best
/// effort). If the client instead disconnects mid-stream (the SSE body future is
/// dropped before either terminal branch runs), [`SseState`]'s own [`Drop`]
/// settles the partial turn off-thread on a best-effort basis, and the
/// active-streams gauge is balanced by [`ActiveStreamGuard`] regardless.
fn sse_response(
    events: TurnEventStream,
    app_state: AppState,
    attribution: UsageAttribution,
    persist: Option<(Uuid, Uuid)>,
    span: tracing::Span,
    started: Instant,
) -> Response {
    // Increment the active-streams gauge now; the guard's `Drop` decrements it on
    // whichever way the stream ends.
    let gauge = ActiveStreamGuard::new(app_state.metrics().clone());
    let initial = SseState {
        events,
        accumulator: TurnAccumulator::new(),
        state: app_state,
        attribution,
        persist,
        span,
        started,
        first_token_seen: false,
        finalized: false,
        done: false,
        _gauge: gauge,
    };

    let body = stream::unfold(initial, |mut state| async move {
        if state.done {
            return None;
        }
        match state.events.next().await {
            Some(Ok(event)) => {
                state.accumulator.ingest(&event);
                // Record first-token latency (request start → first content
                // event) as a span event, once.
                if !state.first_token_seen && is_first_content(&event) {
                    telemetry::record_first_token(&state.span, state.started.elapsed());
                    state.first_token_seen = true;
                }
                let frame = Event::default()
                    .json_data(&event)
                    .unwrap_or_else(|_| Event::default().data("{}"));
                Some((Ok::<Event, Infallible>(frame), state))
            }
            Some(Err(err)) => {
                // Settle whatever was assembled before the failure, then emit a
                // terminal error frame and end the stream.
                state.finalize().await;
                let envelope = ApiError::from_provider(err).envelope();
                let frame = Event::default()
                    .event("error")
                    .json_data(&envelope)
                    .unwrap_or_else(|_| Event::default().event("error").data("{}"));
                state.done = true;
                Some((Ok(frame), state))
            }
            None => {
                state.finalize().await;
                None
            }
        }
    });

    let mut response = Sse::new(body).into_response();
    // Discourage proxy buffering so events flush to the client promptly.
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

impl SseState {
    /// Settles the turn once the stream ends: records a priced usage event
    /// (always) and, for a stateful turn, persists the reassembled assistant
    /// message. Idempotent — a clean end, an error end and the [`Drop`]-path
    /// best-effort settlement cannot both run it, because the first to run sets
    /// the `finalized` flag the others check.
    ///
    /// Best effort throughout: a usage-write or persistence failure is logged,
    /// never surfaced mid-stream.
    ///
    /// # Known limitation: `Message.raw` asymmetry between the turn paths
    ///
    /// The message persisted here has `raw = None`, whereas the non-streaming
    /// path persists `raw = Some(native_payload)` (the provider's verbatim
    /// response blob). This is a deliberate, documented gap — not data loss on
    /// the wire: every SSE frame already carries the verbatim native event in
    /// its [`TurnEvent::raw`](loom_provider::TurnEvent) field, so the streaming
    /// client sees the full native payload. What is missing is only the *single
    /// reconstructed native-response blob* at persistence time. Rebuilding it
    /// would require provider-specific accumulation of native events (a
    /// [`Provider`]-trait change), so it is deferred to a follow-up; the
    /// reassembled [`Message`] uses the provider-agnostic normalised events.
    async fn finalize(&mut self) {
        if self.finalized {
            return;
        }
        self.finalized = true;

        // Usage is finalised from the accumulator's message_delta/turn-end
        // snapshot and recorded for every streamed turn.
        settle_stream_turn(
            self.state.clone(),
            self.attribution.clone(),
            self.persist,
            self.span.clone(),
            self.started.elapsed(),
            self.accumulator.message(),
            self.accumulator.stop_reason_label(),
        )
        .await;
    }
}

impl Drop for SseState {
    /// Best-effort settlement for a stream dropped before [`finalize`] ran — a
    /// mid-stream client disconnect, where hyper drops the SSE body future
    /// without the unfold reaching its clean-end (`None`) or error branch.
    ///
    /// The `finalized` flag guards against double-recording: if [`finalize`]
    /// already ran, this is a no-op (the active-streams gauge is still balanced
    /// by the `_gauge` field's own `Drop`, which runs after this). Otherwise the
    /// partial turn — the work the provider *did* do before the client went away
    /// — is recorded and persisted on a detached task, since `Drop` cannot
    /// `.await`. Handles are cloned out of `self`; the accumulated [`Usage`] is
    /// whatever the stream reported before the disconnect (often empty, as the
    /// usage snapshot rides the terminal event).
    ///
    /// [`finalize`]: SseState::finalize
    fn drop(&mut self) {
        if self.finalized {
            return;
        }
        // Spawning needs a Tokio runtime; if the state is dropped outside one
        // (e.g. during runtime teardown) skip the best-effort persist rather than
        // panic. The gauge is still balanced by `_gauge`.
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        self.finalized = true;
        handle.spawn(settle_stream_turn(
            self.state.clone(),
            self.attribution.clone(),
            self.persist,
            self.span.clone(),
            self.started.elapsed(),
            self.accumulator.message(),
            self.accumulator.stop_reason_label(),
        ));
    }
}

/// Records the priced usage event and provider-duration metric for a streamed
/// turn and, for a stateful turn, persists the reassembled assistant message.
///
/// Shared by [`SseState::finalize`] (the clean/error end) and [`SseState`]'s
/// [`Drop`] (a mid-stream client disconnect) so the two paths record the turn
/// identically. Best effort throughout: a usage-write or persistence failure is
/// logged, never surfaced. Does **not** touch the active-streams gauge — that is
/// owned solely by [`ActiveStreamGuard`].
async fn settle_stream_turn(
    state: AppState,
    attribution: UsageAttribution,
    persist: Option<(Uuid, Uuid)>,
    span: tracing::Span,
    elapsed: Duration,
    message: Message,
    stop_reason: Option<String>,
) {
    let usage = message.usage.clone().unwrap_or_default();
    let cost = record_turn_usage(&state, &attribution, usage.clone()).await;

    // Settle the span kept open across the stream: final usage, cost, stop
    // reason and (only when opted in) the output content.
    telemetry::record_provider_result(&span, &usage, cost, stop_reason.as_deref());
    telemetry::record_output_content(&span, || message_digest(&message));
    state
        .metrics()
        .record_provider_call(&attribution.provider, &attribution.model, true, elapsed);

    if let Some((tenant_id, conversation_id)) = persist {
        let store_span = tracing::info_span!(
            parent: &span,
            "store.append_message",
            "loom.conversation.id" = %conversation_id,
        );
        if let Err(err) = state
            .store()
            .append_message(tenant_id, conversation_id, &message)
            .instrument(store_span)
            .await
        {
            tracing::error!(
                error = %err,
                conversation_id = %conversation_id,
                "failed to persist streamed assistant turn"
            );
        }
    }
}

/// Whether an event carries the first assistant content — the trigger for the
/// first-token latency measurement.
fn is_first_content(event: &TurnEvent) -> bool {
    matches!(
        event.kind,
        TurnEventKind::ContentPartStarted { .. }
            | TurnEventKind::ContentPartDelta { .. }
            | TurnEventKind::ContentPartComplete { .. }
    )
}

/// Reassembles the assistant [`Message`] from a provider's normalised
/// [`TurnEvent`] stream.
///
/// This works over the provider-agnostic [`TurnEventKind`] envelope — completed
/// content parts and the final usage snapshot — so it reassembles any provider's
/// turn identically, without reaching into a provider's native wire format.
#[derive(Default)]
struct TurnAccumulator {
    parts: BTreeMap<usize, loom_core::ContentPart>,
    usage: Option<Usage>,
    stop_reason: Option<loom_provider::StopReason>,
}

impl TurnAccumulator {
    fn new() -> Self {
        Self::default()
    }

    /// Folds one event into the assembled turn.
    fn ingest(&mut self, event: &TurnEvent) {
        match &event.kind {
            TurnEventKind::ContentPartComplete { index, part } => {
                self.parts.insert(*index, part.clone());
            }
            TurnEventKind::Usage(usage) => {
                self.usage = Some(usage.clone());
            }
            TurnEventKind::TurnEnded { stop_reason, usage } => {
                self.stop_reason = Some(stop_reason.clone());
                if let Some(usage) = usage {
                    self.usage = Some(usage.clone());
                }
            }
            _ => {}
        }
    }

    /// The assistant [`Message`] assembled from the events seen so far, with
    /// content parts in ascending index order.
    fn message(&self) -> Message {
        let content = self.parts.values().cloned().collect();
        let mut message = Message::new(Role::Assistant, content);
        message.usage = self.usage.clone();
        message
    }

    /// The turn's stop reason as a telemetry label, if one was reported.
    fn stop_reason_label(&self) -> Option<String> {
        self.stop_reason.as_ref().map(stop_reason_label)
    }
}

/// Registers the `virtual_key` security scheme referenced by every guarded
/// operation.
///
/// The operations declare `security(("virtual_key" = []))`, so the scheme must
/// exist in `components.securitySchemes` for the published document to be a
/// valid, self-consistent OpenAPI spec (no dangling references). It is an HTTP
/// bearer scheme: the tenant presents its virtual key as
/// `Authorization: Bearer loom_...`.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::default);
        components.add_security_scheme(
            "virtual_key",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("virtual key")
                    .build(),
            ),
        );
    }
}

/// The OpenAPI document for the gateway's HTTP surface.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Loom gateway API",
        description = "Multi-tenant LLM gateway: tenant-scoped conversations and turns over pluggable providers."
    ),
    modifiers(&SecurityAddon),
    paths(
        whoami,
        create_conversation,
        get_conversation,
        delete_conversation,
        create_turn,
        stateless_turn,
        usage_rollup,
        crate::batch::create_batch,
        crate::batch::get_batch,
        crate::batch::get_batch_results,
        crate::batch::cancel_batch,
    ),
    components(schemas(
        CreateConversationRequest,
        TurnRequest,
        StatelessTurnRequest,
        crate::batch::CreateBatchRequest,
        crate::batch::BatchItemInput,
        crate::batch::BatchJobDto,
        crate::batch::BatchCountsDto,
    )),
    tags(
        (name = "conversations", description = "Tenant-scoped conversation and turn endpoints"),
        (name = "usage", description = "Spend and token usage rollups"),
        (name = "batches", description = "Asynchronous batch jobs (bulk turns at the discounted batch tier)"),
    )
)]
pub struct ApiDoc;

/// `GET /openapi.json` — the generated OpenAPI 3.x document.
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
