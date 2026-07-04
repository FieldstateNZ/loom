//! `/v1/conversations` handlers: create, fetch (with a page of history), and
//! delete a tenant-scoped conversation.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use loom_core::{Conversation, ProviderBinding};
use loom_store::ConversationStore;

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::state::AppState;

use super::requests::CreateConversationRequest;

/// The default number of messages returned by a conversation fetch.
const DEFAULT_MESSAGE_LIMIT: i64 = 100;
/// The maximum number of messages returned by a single conversation fetch.
const MAX_MESSAGE_LIMIT: i64 = 1000;

/// `POST /v1/conversations` — create a tenant-scoped conversation.
#[utoipa::path(
    post,
    path = "/v1/conversations",
    tag = "conversations",
    request_body = CreateConversationRequest,
    responses(
        (status = 201, description = "Conversation created", body = loom_core::Conversation),
        (status = 400, description = "Malformed request", body = Object),
        (status = 401, description = "Missing or invalid virtual key", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(super) async fn create_conversation(
    State(state): State<AppState>,
    ctx: TenantContext,
    crate::extract::Json(req): crate::extract::Json<CreateConversationRequest>,
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
pub(super) struct Pagination {
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
        (status = 200, description = "The conversation and a page of its messages", body = loom_core::Conversation),
        (status = 404, description = "No such conversation for this tenant", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(super) async fn get_conversation(
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
pub(super) async fn delete_conversation(
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
