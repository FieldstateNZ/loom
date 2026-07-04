//! `/v1/conversations/{id}/turns` and `/v1/turns` handlers: the stateful and
//! stateless turn entry points, sharing [`enforce_limits`] and the
//! [`execute_turn`](super::runner::execute_turn) runner.

use axum::extract::{Path, State};
use axum::response::Response;

use loom_core::{Conversation, ConversationOptions, Message, ProviderBinding, Role};
use loom_provider::{ensure_supported, required_capabilities, Capability, Provider, ProviderError};
use loom_store::ConversationStore;

use crate::auth::TenantContext;
use crate::budget::{self, BudgetWarning};
use crate::error::ApiError;
use crate::state::AppState;

use super::requests::{StatelessTurnRequest, TurnRequest};
use super::runner::execute_turn;

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
pub(super) async fn create_turn(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(id): Path<uuid::Uuid>,
    crate::extract::Json(req): crate::extract::Json<TurnRequest>,
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
pub(super) async fn stateless_turn(
    State(state): State<AppState>,
    ctx: TenantContext,
    crate::extract::Json(req): crate::extract::Json<StatelessTurnRequest>,
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
/// call, shared by the stateful and stateless turn paths — and by batch
/// creation ([`crate::batch::create_batch`]), so an async batch cannot bypass a
/// tenant's budget block or rate limit.
///
/// Returns `Err` with a `429` (rate limit) or `402` (blocked budget); returns
/// `Ok(Some(warning))` when a `warn`-action budget is over its soft limit (the
/// caller surfaces it as the `x-loom-budget-warning` header) and `Ok(None)`
/// otherwise.
pub(crate) async fn enforce_limits(
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
