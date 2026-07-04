//! Persistence for asynchronous batch jobs and their per-item results.

use async_trait::async_trait;
use uuid::Uuid;

use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::model::{BatchCounts, BatchItem, BatchItemStatus, BatchJob, BatchStatus, NewBatchJob};

/// Persistence for asynchronous batch jobs and their per-item results.
///
/// A batch job is a set of stateless turn requests processed asynchronously at
/// the provider's discounted batch tier. Tenant-facing accessors
/// ([`create_batch_job`](Self::create_batch_job),
/// [`get_batch_job`](Self::get_batch_job),
/// [`list_batch_items`](Self::list_batch_items),
/// [`request_batch_cancel`](Self::request_batch_cancel)) are scoped to a
/// `tenant_id`. The poll-worker accessors
/// ([`list_active_batch_jobs`](Self::list_active_batch_jobs),
/// [`get_batch_items`](Self::get_batch_items) and the update methods) are
/// gateway-wide — the worker advances every tenant's jobs — and are keyed by the
/// job's own id, which is an unguessable UUID.
///
/// Per-item results are **stored**, not fetched-through: when a batch ends the
/// worker retrieves the provider's results once and persists each into
/// [`BatchItem::result`], so reads never depend on the provider's results-URL
/// retention.
#[async_trait]
pub trait BatchStore {
    /// Creates a batch job together with its items in one transaction, and
    /// returns the persisted job (status `created`).
    async fn create_batch_job(&self, new: NewBatchJob) -> Result<BatchJob>;

    /// Fetches a job by id, scoped to a tenant. Returns `None` if it does not
    /// exist or belongs to another tenant.
    async fn get_batch_job(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<BatchJob>>;

    /// Lists a job's items in submission order, scoped to a tenant. Returns an
    /// empty vector if the job does not exist or belongs to another tenant.
    async fn list_batch_items(&self, tenant_id: Uuid, batch_id: Uuid) -> Result<Vec<BatchItem>>;

    /// Lists jobs that are still advancing (status other than `ended`),
    /// oldest-first, capped by `limit`. Gateway-wide — for the poll worker only.
    async fn list_active_batch_jobs(&self, limit: i64) -> Result<Vec<BatchJob>>;

    /// Lists a job's items in submission order by job id (not tenant-scoped) —
    /// for the poll worker building the provider submission.
    async fn get_batch_items(&self, batch_id: Uuid) -> Result<Vec<BatchItem>>;

    /// Atomically claims a `created` job for submission, flipping it to
    /// `submitting` (`UPDATE … WHERE id = $1 AND status = 'created'`). Returns
    /// `true` only for the caller that won the claim; a concurrent worker — or a
    /// cancel that already finalised the job — sees `false` and must not submit.
    /// This is the guard that makes the `created → provider` transition
    /// exactly-once even with more than one poll worker (`replicas > 1`).
    ///
    /// Scoped to `tenant_id` (the worker carries the job's tenant) for
    /// defence-in-depth.
    async fn claim_batch_for_submission(&self, tenant_id: Uuid, id: Uuid) -> Result<bool>;

    /// Records that a claimed job was submitted to the provider: stores the
    /// provider-native batch id and initial counts and, **only if the job is
    /// still `submitting`**, moves it to `in_progress`. If a cancellation raced
    /// in and moved the job to `canceling` while it was submitting, the status
    /// is left `canceling` (so the worker relays the cancellation to the
    /// provider) but the provider batch id is still recorded. Guarded on
    /// `status IN ('submitting', 'canceling')` so a finalised (`ended`) job can
    /// never be resurrected. Clears any prior transient error. Scoped to
    /// `tenant_id`.
    async fn mark_batch_submitted(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        provider_batch_id: &str,
        counts: BatchCounts,
    ) -> Result<()>;

    /// Releases a claim after a failed provider submission: reverts a
    /// `submitting` job back to `created` (guarded `WHERE status = 'submitting'`)
    /// and records `error`, so the job is retried on a later pass rather than
    /// stranded in `submitting`. A no-op if the job is no longer `submitting`
    /// (e.g. a cancellation moved it to `canceling`). Scoped to `tenant_id`.
    async fn release_batch_submission(&self, tenant_id: Uuid, id: Uuid, error: &str) -> Result<()>;

    /// Applies a poll result: updates counts and status, and — when the job has
    /// ended — records the `results_url` and `ended_at`. Clears any prior
    /// transient error. Scoped to `tenant_id`.
    async fn update_batch_progress(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        status: BatchStatus,
        counts: BatchCounts,
        results_url: Option<&str>,
        ended_at: Option<DateTime<Utc>>,
    ) -> Result<()>;

    /// Finalises a job as `ended`/canceled locally without any provider
    /// round-trip: marks every still-pending item canceled and sets
    /// `canceled_count = total_items`. Used both when a `created` job is
    /// cancelled before submission and when a cancel-during-submission never
    /// produced a provider batch. Idempotent (guarded `WHERE status <> 'ended'`)
    /// and scoped to `tenant_id`.
    async fn finalize_batch_canceled(&self, tenant_id: Uuid, id: Uuid) -> Result<()>;

    /// Persists one item's resolved result (status + payload), keyed by
    /// `(batch_id, custom_id)` and scoped to `tenant_id`.
    async fn save_batch_item_result(
        &self,
        tenant_id: Uuid,
        batch_id: Uuid,
        custom_id: &str,
        status: BatchItemStatus,
        result: &serde_json::Value,
    ) -> Result<()>;

    /// Requests cancellation of a tenant's job.
    ///
    /// A `created` job (never submitted) is finalised immediately as `ended`
    /// with every item canceled. A job that is `submitting` (claimed by the
    /// worker but not yet live) or `in_progress` moves to `canceling`, so the
    /// worker relays the cancellation to the provider instead of the job going
    /// live — critically, a `submitting` job is **not** finalised locally, which
    /// would let a concurrent `mark_batch_submitted` clobber it back to running.
    /// Returns the updated job, or `None` if it does not exist or belongs to
    /// another tenant. An already-`ended` job is returned unchanged (idempotent).
    async fn request_batch_cancel(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<BatchJob>>;

    /// Records a transient provider/poll error against a job **without** changing
    /// its status, so a provider fault never corrupts the lifecycle. Scoped to
    /// `tenant_id`.
    async fn set_batch_error(&self, tenant_id: Uuid, id: Uuid, error: &str) -> Result<()>;
}
