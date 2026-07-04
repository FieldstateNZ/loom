//! `POST /v1/batches` — create a batch from a list of items.

use std::collections::BTreeSet;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use http::StatusCode;

use loom_core::{Conversation, ConversationOptions};
use loom_provider::{
    ensure_supported, required_capabilities, Capability, ProviderDescriptor, ProviderError,
};
use loom_store::{BatchStore, NewBatchItem, NewBatchJob};

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::extract;
use crate::state::AppState;

use super::conversation_from;
use super::dto::{BatchJobDto, CreateBatchRequest};

/// The maximum number of items accepted in a single batch.
const MAX_BATCH_ITEMS: usize = 100_000;

/// Runs capability negotiation for one batch item, requiring
/// [`Capability::Batches`] on top of the item's own required capabilities.
fn negotiate_batch_item(
    descriptor: &ProviderDescriptor,
    conversation: &Conversation,
    options: &ConversationOptions,
) -> Result<(), ApiError> {
    let model = descriptor
        .model(&conversation.binding.model)
        .ok_or_else(|| {
            ApiError::from_provider(ProviderError::ModelNotFound {
                provider: descriptor.name.clone(),
                model: conversation.binding.model.clone(),
            })
        })?;
    let mut required = required_capabilities(conversation, options);
    required.insert(Capability::Batches);
    ensure_supported(&descriptor.name, model, &required).map_err(ApiError::from_provider)
}

/// `POST /v1/batches` — create a batch from a list of items.
///
/// Budget and rate limits are enforced up front with the **same** check the
/// interactive turn paths use ([`crate::v1::enforce_limits`]), so a tenant whose
/// budget action is `block` cannot bypass the block by submitting work as a
/// batch. The rate-limit check counts a batch create as a **single** request
/// (not one per item); the budget block is the primary guard against a large
/// batch running up spend past a hard cap.
#[utoipa::path(
    post,
    path = "/v1/batches",
    tag = "batches",
    request_body = CreateBatchRequest,
    responses(
        (status = 201, description = "Batch created (status = created)", body = Object),
        (status = 400, description = "Malformed request", body = Object),
        (status = 401, description = "Missing or invalid virtual key", body = Object),
        (status = 402, description = "Budget exceeded (block action)", body = Object),
        (status = 422, description = "Capability unsupported or provider not configured", body = Object),
        (status = 429, description = "Rate limit exceeded", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(crate) async fn create_batch(
    State(state): State<AppState>,
    ctx: TenantContext,
    extract::Json(req): extract::Json<CreateBatchRequest>,
) -> Result<Response, ApiError> {
    if req.items.is_empty() {
        return Err(ApiError::bad_request("items must not be empty"));
    }
    if req.items.len() > MAX_BATCH_ITEMS {
        return Err(ApiError::bad_request(format!(
            "a batch may contain at most {MAX_BATCH_ITEMS} items"
        )));
    }

    let provider_name = req.items[0].provider.trim().to_owned();
    if provider_name.is_empty() {
        return Err(ApiError::bad_request("provider must not be empty"));
    }

    // Enforce rate limit + budget before accepting the batch — the same preflight
    // as an interactive turn, so a `block` budget cannot be sidestepped via a
    // batch. A `warn`-action budget over its soft limit is surfaced as a header.
    let warning = crate::v1::enforce_limits(&state, &ctx).await?;

    // Resolve the provider once for capability negotiation across the batch.
    let provider = state
        .resolve_provider(ctx.tenant_id, &provider_name)
        .await?;
    let descriptor = provider.descriptor();

    let mut custom_ids = BTreeSet::new();
    let mut new_items = Vec::with_capacity(req.items.len());
    for (index, item) in req.items.iter().enumerate() {
        if item.provider.trim() != provider_name {
            return Err(ApiError::bad_request(
                "all items in a batch must target the same provider",
            ));
        }
        if item.model.trim().is_empty() {
            return Err(ApiError::bad_request("model must not be empty"));
        }
        if item.messages.is_empty() {
            return Err(ApiError::bad_request("messages must not be empty"));
        }

        let (conversation, options) = conversation_from(ctx.tenant_id, item);
        negotiate_batch_item(&descriptor, &conversation, &options)?;

        let custom_id = match &item.custom_id {
            Some(id) if !id.trim().is_empty() => id.clone(),
            Some(_) => return Err(ApiError::bad_request("custom_id must not be blank")),
            None => format!("item-{index}"),
        };
        if !custom_ids.insert(custom_id.clone()) {
            return Err(ApiError::bad_request(format!(
                "duplicate custom_id {custom_id:?}"
            )));
        }

        let request = serde_json::to_value(item).map_err(|_| ApiError::internal())?;
        new_items.push(NewBatchItem {
            custom_id,
            model: item.model.clone(),
            request,
        });
    }

    let job = state
        .store()
        .create_batch_job(NewBatchJob {
            tenant_id: ctx.tenant_id,
            virtual_key_id: Some(ctx.key_id),
            provider: provider_name,
            items: new_items,
        })
        .await
        .map_err(ApiError::from_store)?;

    let mut response = (StatusCode::CREATED, Json(BatchJobDto::from(job))).into_response();
    // A `warn`-action budget over its soft limit lets creation proceed but flags
    // it to the caller, mirroring the interactive turn paths.
    if let Some(warning) = warning {
        if let Ok(value) = http::HeaderValue::from_str(warning.header_value()) {
            response
                .headers_mut()
                .insert("x-loom-budget-warning", value);
        }
    }
    Ok(response)
}
