//! Asynchronous **batch** processing: the `/v1/batches` API, the provider-batch
//! seam, and the poll worker.
//!
//! A [`BatchJob`](loom_store::BatchJob) is a set of stateless turn requests —
//! each the inline `{provider, model, system?, messages, options?}` shape of
//! `POST /v1/turns` — submitted together and processed asynchronously at the
//! provider's discounted batch tier. The lifecycle is
//! `created → submitting → in_progress → ended`; a cancellation passes through
//! `canceling`. The `submitting` step is an **atomic claim**: the worker flips
//! `created → submitting` before it calls the provider, so a given job is
//! submitted exactly once even with more than one worker (`replicas > 1`).
//!
//! # Pieces
//!
//! - The HTTP API ([`create_batch`], [`get_batch`], [`get_batch_results`],
//!   [`cancel_batch`]) is tenant-scoped and scopes every store call to the
//!   caller's tenant. [`create_batch`] runs the same budget/rate-limit preflight
//!   as an interactive turn, so a batch cannot bypass a tenant's spend block.
//! - The [`BatchBackend`] trait is the provider seam the worker drives; the
//!   [`DefaultBatchBackendFactory`] resolves the tenant's credential and builds
//!   an Anthropic-backed backend, while tests inject a fake backend through
//!   [`AppState::with_batch_backend_factory`](crate::state::AppState::with_batch_backend_factory).
//! - [`run_batch_poll_pass`] performs **one** advance-everything pass over the
//!   active jobs — submitting `created` jobs, polling `in_progress`/`canceling`
//!   ones, and finalising them with per-item results and priced (batch-tier)
//!   usage. It takes no wall-clock dependency, so a test can drive the whole
//!   lifecycle by calling it repeatedly; [`spawn_batch_worker`] just calls it on
//!   a fixed interval.
//!
//! # Layout
//!
//! - [`submit_item`] — [`BatchSubmitItem`], the provider-agnostic unit of work.
//! - [`snapshot`] — [`ProviderBatchSnapshot`] and [`ProviderBatchResult`].
//! - [`backend`] — the [`BatchBackend`]/[`BatchBackendFactory`] provider seam.
//! - [`anthropic_backend`] — the default, Anthropic-backed implementation.
//! - [`poll`] — [`run_batch_poll_pass`] and the per-job state dispatch.
//! - [`finalize`] — building a submission and finalising an ended job.
//! - [`worker`] — [`spawn_batch_worker`], the background poll loop.
//! - [`dto`] — the HTTP request/response DTOs.
//! - [`routes`] — the sub-router and the fetch/results/cancel handlers.
//! - [`create`] — the `create_batch` handler (the largest one, split out on
//!   its own).

mod anthropic_backend;
mod backend;
mod create;
mod dto;
mod finalize;
mod poll;
mod routes;
mod snapshot;
mod submit_item;
mod worker;

pub use anthropic_backend::DefaultBatchBackendFactory;
pub use backend::{BatchBackend, BatchBackendFactory};
pub use dto::{BatchCountsDto, BatchItemInput, BatchJobDto, CreateBatchRequest};
pub use poll::{run_batch_poll_pass, PollReport};
pub use routes::router;
pub use snapshot::{ProviderBatchResult, ProviderBatchSnapshot};
pub use submit_item::BatchSubmitItem;
pub use worker::spawn_batch_worker;

// `create_batch`, `get_batch`, `get_batch_results` and `cancel_batch` are
// `pub(crate)` in `create`/`routes`, re-exported here via glob rather than by
// name: `#[utoipa::path]` also emits a hidden `__path_<fn>` marker type
// alongside each handler, in the same module, and `v1::ApiDoc`'s `paths(...)`
// list resolves those via the handler's *re-exported* path
// (`crate::batch::create_batch`, …) — so the markers must be reachable at
// this path too. The glob picks up both without widening any handler's own
// visibility.
pub(crate) use create::*;
pub(crate) use routes::*;

use uuid::Uuid;

use loom_core::{Conversation, ConversationOptions, ProviderBinding};

/// Builds a [`Conversation`] and options from a stored/submitted batch item.
///
/// Shared by the [`poll`] worker (reconstructing a submitted item) and the
/// [`routes`] HTTP handler (negotiating capabilities up front).
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
