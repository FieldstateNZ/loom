//! Asynchronous **batch** processing: the `/v1/batches` API, the provider-batch
//! seam, and the poll worker.
//!
//! A [`BatchJob`](loom_store::BatchJob) is a set of stateless turn requests —
//! each the inline `{provider, model, system?, messages, options?}` shape of
//! `POST /v1/turns` — submitted together and processed asynchronously at the
//! provider's discounted batch tier. The lifecycle is
//! `created → in_progress → ended`; a cancellation passes through `canceling`.
//!
//! # Pieces
//!
//! - The HTTP API ([`create_batch`], [`get_batch`], [`get_batch_results`],
//!   [`cancel_batch`]) is tenant-scoped and scopes every store call to the
//!   caller's tenant.
//! - The [`BatchBackend`] trait is the provider seam the worker drives; the
//!   [`DefaultBatchBackendFactory`] resolves the tenant's credential and builds
//!   an Anthropic-backed backend, while tests inject a fake backend through
//!   [`AppState::with_batch_backend_factory`].
//! - [`run_batch_poll_pass`] performs **one** advance-everything pass over the
//!   active jobs — submitting `created` jobs, polling `in_progress`/`canceling`
//!   ones, and finalising them with per-item results and priced (batch-tier)
//!   usage. It takes no wall-clock dependency, so a test can drive the whole
//!   lifecycle by calling it repeatedly; [`spawn_batch_worker`] just calls it on
//!   a fixed interval.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use futures::stream;
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;
use uuid::Uuid;

use loom_core::{CacheHint, Conversation, ConversationOptions, Message, ProviderBinding, Usage};
use loom_provider::{
    ensure_supported, required_capabilities, Capability, ProviderDescriptor, ProviderError,
};
use loom_provider_anthropic::{
    translate, AnthropicProvider, BatchRequest, PROVIDER_NAME as ANTHROPIC_PROVIDER,
};
use loom_store::{
    BatchCounts, BatchItem, BatchItemStatus, BatchJob, BatchStatus, BatchStore, NewBatchItem,
    NewBatchJob, NewUsageEvent, Pricer, PricingStore,
};

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::extract;
use crate::state::AppState;

/// The maximum number of items accepted in a single batch.
const MAX_BATCH_ITEMS: usize = 100_000;

/// The number of active jobs the worker advances per poll pass.
const BATCH_POLL_LIMIT: i64 = 256;

// ===========================================================================
// Provider seam
// ===========================================================================

/// One item to submit to a provider batch: a correlation id plus the
/// (provider-agnostic) conversation to run. Translation to the provider's native
/// request shape happens inside the [`BatchBackend`].
#[derive(Clone, Debug)]
pub struct BatchSubmitItem {
    /// The per-item correlation id, echoed on the matching result.
    pub custom_id: String,
    /// The conversation to run.
    pub conversation: Conversation,
    /// The request-time options.
    pub options: ConversationOptions,
}

/// A provider-agnostic snapshot of a batch's state.
#[derive(Clone, Debug)]
pub struct ProviderBatchSnapshot {
    /// The provider-native batch id.
    pub provider_batch_id: String,
    /// Whether the batch has reached its terminal state.
    pub ended: bool,
    /// Per-status item counts.
    pub counts: BatchCounts,
    /// The results document location, once ended.
    pub results_url: Option<String>,
}

/// A provider-agnostic per-item result.
#[derive(Clone, Debug)]
pub struct ProviderBatchResult {
    /// The correlation id of the request this result belongs to.
    pub custom_id: String,
    /// The item's terminal outcome.
    pub outcome: BatchItemStatus,
    /// The verbatim result payload to persist (assistant message on success, or
    /// the provider error otherwise).
    pub result: Value,
    /// The parsed usage snapshot, for billing (present on success).
    pub usage: Option<Usage>,
}

/// A provider's batch surface: submit, poll, fetch results, cancel.
///
/// The seam the [poll worker](run_batch_poll_pass) drives. The production
/// implementation wraps [`AnthropicProvider`]'s batch methods; tests inject a
/// deterministic fake so the whole lifecycle runs without a live API or real
/// time.
#[async_trait]
pub trait BatchBackend: Send + Sync {
    /// Submits `items` as a new provider batch.
    async fn submit(
        &self,
        items: Vec<BatchSubmitItem>,
    ) -> Result<ProviderBatchSnapshot, ProviderError>;

    /// Polls the current state of the provider batch `provider_batch_id`.
    async fn poll(&self, provider_batch_id: &str) -> Result<ProviderBatchSnapshot, ProviderError>;

    /// Retrieves the per-item results for an ended `snapshot`.
    async fn results(
        &self,
        snapshot: &ProviderBatchSnapshot,
    ) -> Result<Vec<ProviderBatchResult>, ProviderError>;

    /// Requests cancellation of the provider batch `provider_batch_id`,
    /// returning the updated snapshot.
    async fn cancel(&self, provider_batch_id: &str)
        -> Result<ProviderBatchSnapshot, ProviderError>;
}

/// Resolves a [`BatchBackend`] for a `(tenant, provider)` pair.
///
/// Mirrors [`ProviderFactory`](crate::provider::ProviderFactory): credential
/// loading and decryption happen here so the worker never touches secrets.
#[async_trait]
pub trait BatchBackendFactory: Send + Sync {
    /// Resolves `provider` for `tenant_id`, returning a shared backend handle.
    async fn backend(
        &self,
        state: &AppState,
        tenant_id: Uuid,
        provider: &str,
    ) -> Result<Arc<dyn BatchBackend>, ApiError>;
}

/// The default factory over the providers compiled into the gateway. Recognises
/// `"anthropic"`; any other name is a `422`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultBatchBackendFactory;

#[async_trait]
impl BatchBackendFactory for DefaultBatchBackendFactory {
    async fn backend(
        &self,
        state: &AppState,
        tenant_id: Uuid,
        provider: &str,
    ) -> Result<Arc<dyn BatchBackend>, ApiError> {
        match provider {
            ANTHROPIC_PROVIDER => {
                let credential =
                    crate::provider::load_credential(state, tenant_id, provider).await?;
                let api_key = crate::provider::decrypt_api_key(state, &credential)?;
                let mut anthropic =
                    AnthropicProvider::new(api_key).map_err(ApiError::from_provider)?;
                if let Some(base_url) = credential.base_url {
                    anthropic = anthropic.with_base_url(base_url);
                }
                Ok(Arc::new(AnthropicBatchBackend {
                    provider: anthropic,
                }))
            }
            other => Err(ApiError::unprocessable(
                "unknown_provider",
                format!("provider {other:?} does not support batches on this gateway"),
            )),
        }
    }
}

/// The Anthropic-backed [`BatchBackend`]: translates each item to a native
/// Messages request and drives Anthropic's Message Batches API.
struct AnthropicBatchBackend {
    provider: AnthropicProvider,
}

impl AnthropicBatchBackend {
    /// Maps a native Anthropic batch snapshot to the provider-agnostic shape.
    fn snapshot(batch: &loom_provider_anthropic::AnthropicBatch) -> ProviderBatchSnapshot {
        let clamp = |v: i64| i32::try_from(v.max(0)).unwrap_or(i32::MAX);
        ProviderBatchSnapshot {
            provider_batch_id: batch.id.clone(),
            ended: batch.is_ended(),
            counts: BatchCounts {
                processing: clamp(batch.counts.processing),
                succeeded: clamp(batch.counts.succeeded),
                errored: clamp(batch.counts.errored),
                canceled: clamp(batch.counts.canceled),
                expired: clamp(batch.counts.expired),
            },
            results_url: batch.results_url.clone(),
        }
    }
}

#[async_trait]
impl BatchBackend for AnthropicBatchBackend {
    async fn submit(
        &self,
        items: Vec<BatchSubmitItem>,
    ) -> Result<ProviderBatchSnapshot, ProviderError> {
        let requests: Vec<BatchRequest> = items
            .iter()
            .map(|item| BatchRequest {
                custom_id: item.custom_id.clone(),
                params: translate::translate_request(&item.conversation, &item.options),
            })
            .collect();
        let batch = self.provider.create_batch(&requests).await?;
        Ok(Self::snapshot(&batch))
    }

    async fn poll(&self, provider_batch_id: &str) -> Result<ProviderBatchSnapshot, ProviderError> {
        let batch = self.provider.get_batch(provider_batch_id).await?;
        Ok(Self::snapshot(&batch))
    }

    async fn results(
        &self,
        snapshot: &ProviderBatchSnapshot,
    ) -> Result<Vec<ProviderBatchResult>, ProviderError> {
        let Some(url) = snapshot.results_url.as_deref() else {
            return Ok(Vec::new());
        };
        let raw = self.provider.fetch_batch_results(url).await?;
        Ok(raw
            .into_iter()
            .map(|r| {
                let outcome = match r.result.get("type").and_then(Value::as_str) {
                    Some("succeeded") => BatchItemStatus::Succeeded,
                    Some("canceled") => BatchItemStatus::Canceled,
                    Some("expired") => BatchItemStatus::Expired,
                    // Anything else (including "errored") is a failure.
                    _ => BatchItemStatus::Errored,
                };
                let usage = if outcome == BatchItemStatus::Succeeded {
                    r.result
                        .get("message")
                        .and_then(|m| m.get("usage"))
                        .map(translate::translate_usage)
                } else {
                    None
                };
                ProviderBatchResult {
                    custom_id: r.custom_id,
                    outcome,
                    result: r.result,
                    usage,
                }
            })
            .collect())
    }

    async fn cancel(
        &self,
        provider_batch_id: &str,
    ) -> Result<ProviderBatchSnapshot, ProviderError> {
        let batch = self.provider.cancel_batch(provider_batch_id).await?;
        Ok(Self::snapshot(&batch))
    }
}

// ===========================================================================
// Poll worker
// ===========================================================================

/// The outcome of a single [`run_batch_poll_pass`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollReport {
    /// Jobs that changed state (submitted, ended, …) this pass.
    pub advanced: usize,
    /// Jobs that hit a transient provider/poll error (state left intact).
    pub errored: usize,
}

/// Advances every active batch job by one step: submits `created` jobs, polls
/// `in_progress`/`canceling` ones, and finalises those the provider reports as
/// ended.
///
/// Best-effort and non-corrupting: a provider or store failure on one job is
/// recorded against that job (via [`BatchStore::set_batch_error`]) and the pass
/// continues; the job stays in its current state and is retried next pass.
///
/// Returns a [`PollReport`] so tests and the worker loop can observe progress.
pub async fn run_batch_poll_pass(state: &AppState) -> PollReport {
    let jobs = match state.store().list_active_batch_jobs(BATCH_POLL_LIMIT).await {
        Ok(jobs) => jobs,
        Err(err) => {
            tracing::warn!(error = %err, "batch poll: listing active jobs failed");
            return PollReport::default();
        }
    };

    let mut report = PollReport::default();
    for job in jobs {
        match advance_job(state, &job).await {
            Ok(true) => report.advanced += 1,
            Ok(false) => {}
            Err(err) => {
                report.errored += 1;
                tracing::warn!(batch_id = %job.id, error = %err, "batch poll: job advance failed");
                // Record the transient error without changing status, so the
                // lifecycle is not corrupted and the job is retried next pass.
                if let Err(store_err) = state.store().set_batch_error(job.id, &err).await {
                    tracing::error!(batch_id = %job.id, error = %store_err, "batch poll: recording error failed");
                }
            }
        }
    }
    report
}

/// Advances one job. Returns `Ok(true)` if the job changed state.
async fn advance_job(state: &AppState, job: &BatchJob) -> Result<bool, String> {
    let backend = state
        .batch_backend_factory()
        .backend(state, job.tenant_id, &job.provider)
        .await
        .map_err(|e| format!("resolve backend: {}", e.code()))?;

    match job.status {
        BatchStatus::Created => {
            let items = state
                .store()
                .get_batch_items(job.id)
                .await
                .map_err(|e| format!("load items: {e}"))?;
            let submit = build_submit_items(job.tenant_id, &items)?;
            let snapshot = backend
                .submit(submit)
                .await
                .map_err(|e| format!("submit: {e}"))?;
            state
                .store()
                .mark_batch_submitted(job.id, &snapshot.provider_batch_id, snapshot.counts)
                .await
                .map_err(|e| format!("mark submitted: {e}"))?;
            Ok(true)
        }
        BatchStatus::InProgress => {
            let provider_batch_id = job
                .provider_batch_id
                .as_deref()
                .ok_or_else(|| "in-progress job has no provider batch id".to_owned())?;
            let snapshot = backend
                .poll(provider_batch_id)
                .await
                .map_err(|e| format!("poll: {e}"))?;
            if snapshot.ended {
                finalize_job(state, job, backend.as_ref(), &snapshot).await?;
                Ok(true)
            } else {
                state
                    .store()
                    .update_batch_progress(
                        job.id,
                        BatchStatus::InProgress,
                        snapshot.counts,
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| format!("update progress: {e}"))?;
                Ok(false)
            }
        }
        BatchStatus::Canceling => {
            let provider_batch_id = job
                .provider_batch_id
                .as_deref()
                .ok_or_else(|| "canceling job has no provider batch id".to_owned())?;
            // Relay the cancellation (idempotent) and observe the result.
            let snapshot = backend
                .cancel(provider_batch_id)
                .await
                .map_err(|e| format!("cancel: {e}"))?;
            if snapshot.ended {
                finalize_job(state, job, backend.as_ref(), &snapshot).await?;
                Ok(true)
            } else {
                state
                    .store()
                    .update_batch_progress(
                        job.id,
                        BatchStatus::Canceling,
                        snapshot.counts,
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| format!("update progress: {e}"))?;
                Ok(false)
            }
        }
        // Filtered out by `list_active_batch_jobs`; nothing to do.
        BatchStatus::Ended => Ok(false),
    }
}

/// Builds the provider submission from stored items, reconstructing each turn.
fn build_submit_items(
    tenant_id: Uuid,
    items: &[BatchItem],
) -> Result<Vec<BatchSubmitItem>, String> {
    items
        .iter()
        .map(|item| {
            let input: BatchItemInput = serde_json::from_value(item.request.clone())
                .map_err(|e| format!("decode stored item {}: {e}", item.custom_id))?;
            let (conversation, options) = conversation_from(tenant_id, &input);
            Ok(BatchSubmitItem {
                custom_id: item.custom_id.clone(),
                conversation,
                options,
            })
        })
        .collect()
}

/// Finalises an ended job: fetches results, persists each item's outcome,
/// records batch-tier usage for the succeeded items, and marks the job ended.
async fn finalize_job(
    state: &AppState,
    job: &BatchJob,
    backend: &dyn BatchBackend,
    snapshot: &ProviderBatchSnapshot,
) -> Result<(), String> {
    let results = backend
        .results(snapshot)
        .await
        .map_err(|e| format!("fetch results: {e}"))?;

    // Map custom_id → model so batch usage is priced against the right model.
    let items = state
        .store()
        .get_batch_items(job.id)
        .await
        .map_err(|e| format!("load items: {e}"))?;
    let model_for = |custom_id: &str| -> Option<String> {
        items
            .iter()
            .find(|i| i.custom_id == custom_id)
            .map(|i| i.model.clone())
    };

    for result in &results {
        state
            .store()
            .save_batch_item_result(job.id, &result.custom_id, result.outcome, &result.result)
            .await
            .map_err(|e| format!("save item result: {e}"))?;

        if let (BatchItemStatus::Succeeded, Some(usage), Some(model)) = (
            result.outcome,
            result.usage.clone(),
            model_for(&result.custom_id),
        ) {
            record_batch_usage(state, job, &model, usage).await;
        }
    }

    state
        .store()
        .update_batch_progress(
            job.id,
            BatchStatus::Ended,
            snapshot.counts,
            snapshot.results_url.as_deref(),
            Some(Utc::now()),
        )
        .await
        .map_err(|e| format!("finalize: {e}"))?;
    Ok(())
}

/// Records a priced, batch-tier usage event for one succeeded item (best
/// effort, via the state's outbox-backed recorder).
async fn record_batch_usage(state: &AppState, job: &BatchJob, model: &str, usage: Usage) {
    let cost = match state
        .store()
        .get_effective_price(&job.provider, model, Utc::now())
        .await
    {
        Ok(Some(price)) => Some(Pricer::cost_with_mode(&usage, &price, true)),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(error = %err, "batch price lookup failed; recording usage without cost");
            None
        }
    };
    state.metrics().record_tokens(
        job.tenant_id,
        model,
        usage.input_tokens.unwrap_or(0),
        usage.output_tokens.unwrap_or(0),
    );
    if let Some(cost) = cost {
        state.metrics().record_cost(job.tenant_id, model, cost);
    }
    let event = NewUsageEvent {
        tenant_id: job.tenant_id,
        virtual_key_id: job.virtual_key_id,
        conversation_id: None,
        provider: job.provider.clone(),
        model: model.to_owned(),
        usage,
        cost,
        is_batch: true,
    };
    state.usage_recorder().record(state.store(), event).await;
}

/// Spawns the background poll worker, advancing active batches every `interval`.
///
/// A no-op forever-pending task when `interval` is zero (worker disabled). The
/// returned handle is detached by the caller; the worker exits when the process
/// does.
#[must_use]
pub fn spawn_batch_worker(state: AppState, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if interval.is_zero() {
            return;
        }
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let report = run_batch_poll_pass(&state).await;
            if report.advanced > 0 || report.errored > 0 {
                tracing::debug!(
                    advanced = report.advanced,
                    errored = report.errored,
                    "batch poll pass complete"
                );
            }
        }
    })
}

// ===========================================================================
// HTTP API
// ===========================================================================

/// Builds a [`Conversation`] and options from a stored/submitted batch item.
fn conversation_from(
    tenant_id: Uuid,
    input: &BatchItemInput,
) -> (Conversation, ConversationOptions) {
    let options = input.options.clone().unwrap_or_default();
    let mut conversation = Conversation::new(
        tenant_id,
        ProviderBinding::new(input.provider.clone(), input.model.clone()),
    );
    conversation.system = input.system.clone();
    conversation.system_cache = input.system_cache;
    conversation.messages = input.messages.clone();
    (conversation, options)
}

/// One item of a batch create request: the inline stateless-turn shape plus an
/// optional caller correlation id.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct BatchItemInput {
    /// A caller-facing correlation id, unique within the batch. Defaults to
    /// `item-{index}` when omitted.
    #[serde(default)]
    pub custom_id: Option<String>,
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
}

/// Request body for creating a batch.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBatchRequest {
    /// The batch's items, in submission order.
    pub items: Vec<BatchItemInput>,
}

/// Per-status item counts in a batch response.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchCountsDto {
    /// Items still being processed.
    pub processing: i32,
    /// Items that completed successfully.
    pub succeeded: i32,
    /// Items that failed.
    pub errored: i32,
    /// Items that were canceled.
    pub canceled: i32,
    /// Items that expired.
    pub expired: i32,
}

impl From<BatchCounts> for BatchCountsDto {
    fn from(c: BatchCounts) -> Self {
        Self {
            processing: c.processing,
            succeeded: c.succeeded,
            errored: c.errored,
            canceled: c.canceled,
            expired: c.expired,
        }
    }
}

/// A batch job as returned by the API.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchJobDto {
    /// The job id.
    pub id: Uuid,
    /// The provider the job runs against.
    pub provider: String,
    /// The lifecycle status (`created`, `in_progress`, `canceling`, `ended`).
    pub status: String,
    /// The provider-native batch id, once submitted.
    pub provider_batch_id: Option<String>,
    /// The total number of items.
    pub total_items: i32,
    /// Per-status item counts.
    pub counts: BatchCountsDto,
    /// The last transient provider/poll error, if any.
    pub error: Option<String>,
    /// When the job was created.
    pub created_at: DateTime<Utc>,
    /// When the job was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the job ended, if it has.
    pub ended_at: Option<DateTime<Utc>>,
}

impl From<BatchJob> for BatchJobDto {
    fn from(job: BatchJob) -> Self {
        Self {
            id: job.id,
            provider: job.provider,
            status: job.status.as_str().to_owned(),
            provider_batch_id: job.provider_batch_id,
            total_items: job.total_items,
            counts: job.counts.into(),
            error: job.error,
            created_at: job.created_at,
            updated_at: job.updated_at,
            ended_at: job.ended_at,
        }
    }
}

/// Builds the `/v1/batches` sub-router (tenant auth is applied by the parent).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/batches", post(create_batch))
        .route("/v1/batches/{id}", get(get_batch))
        .route("/v1/batches/{id}/results", get(get_batch_results))
        .route("/v1/batches/{id}/cancel", post(cancel_batch))
}

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
#[utoipa::path(
    post,
    path = "/v1/batches",
    tag = "batches",
    request_body = CreateBatchRequest,
    responses(
        (status = 201, description = "Batch created (status = created)", body = Object),
        (status = 400, description = "Malformed request", body = Object),
        (status = 401, description = "Missing or invalid virtual key", body = Object),
        (status = 422, description = "Capability unsupported or provider not configured", body = Object),
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

    Ok((StatusCode::CREATED, Json(BatchJobDto::from(job))).into_response())
}

/// `GET /v1/batches/{id}` — the batch's status and counts.
#[utoipa::path(
    get,
    path = "/v1/batches/{id}",
    tag = "batches",
    params(("id" = Uuid, Path, description = "Batch id")),
    responses(
        (status = 200, description = "The batch status and counts", body = Object),
        (status = 404, description = "No such batch for this tenant", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(crate) async fn get_batch(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let job = state
        .store()
        .get_batch_job(ctx.tenant_id, id)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::not_found("batch not found"))?;
    Ok(Json(BatchJobDto::from(job)))
}

/// `GET /v1/batches/{id}/results` — the per-item results as streamed JSONL
/// (one `{custom_id, status, result}` object per line).
#[utoipa::path(
    get,
    path = "/v1/batches/{id}/results",
    tag = "batches",
    params(("id" = Uuid, Path, description = "Batch id")),
    responses(
        (status = 200, description = "JSONL stream, one result object per line", body = String),
        (status = 404, description = "No such batch for this tenant", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(crate) async fn get_batch_results(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    // Confirm the tenant owns the batch (a missing/foreign batch is a 404).
    let job = state
        .store()
        .get_batch_job(ctx.tenant_id, id)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::not_found("batch not found"))?;

    let items = state
        .store()
        .list_batch_items(ctx.tenant_id, job.id)
        .await
        .map_err(ApiError::from_store)?;

    // One newline-terminated JSON object per item. Built up front (the item set
    // is bounded by the batch the caller submitted) and delivered as a stream.
    let lines: Vec<Result<String, std::convert::Infallible>> = items
        .into_iter()
        .map(|item| {
            let line = json!({
                "custom_id": item.custom_id,
                "status": item.status.as_str(),
                "result": item.result,
            });
            Ok(format!("{line}\n"))
        })
        .collect();

    let body = Body::from_stream(stream::iter(lines));
    let mut response = body.into_response();
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/x-ndjson"),
    );
    Ok(response)
}

/// `POST /v1/batches/{id}/cancel` — request cancellation of a batch.
#[utoipa::path(
    post,
    path = "/v1/batches/{id}/cancel",
    tag = "batches",
    params(("id" = Uuid, Path, description = "Batch id")),
    responses(
        (status = 200, description = "The batch, transitioned toward cancellation", body = Object),
        (status = 404, description = "No such batch for this tenant", body = Object),
    ),
    security(("virtual_key" = []))
)]
pub(crate) async fn cancel_batch(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let job = state
        .store()
        .request_batch_cancel(ctx.tenant_id, id)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::not_found("batch not found"))?;
    Ok(Json(BatchJobDto::from(job)))
}
