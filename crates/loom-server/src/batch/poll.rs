//! The poll worker's core pass: [`run_batch_poll_pass`] advances every active
//! batch job by one lifecycle step.

use loom_store::{BatchJob, BatchStatus, BatchStore};

use crate::state::AppState;

use super::finalize::{build_submit_items, finalize_job};

/// The number of active jobs the worker advances per poll pass.
const BATCH_POLL_LIMIT: i64 = 256;

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
                if let Err(store_err) = state
                    .store()
                    .set_batch_error(job.tenant_id, job.id, &err)
                    .await
                {
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
            // Atomically claim `created → submitting` FIRST. Only the worker that
            // wins the claim submits; a concurrent worker (or a cancel that
            // already finalised the job) matches no row and skips. This is what
            // makes provider submission exactly-once under `replicas > 1`.
            let claimed = state
                .store()
                .claim_batch_for_submission(job.tenant_id, job.id)
                .await
                .map_err(|e| format!("claim: {e}"))?;
            if !claimed {
                return Ok(false);
            }
            let items = state
                .store()
                .get_batch_items(job.id)
                .await
                .map_err(|e| format!("load items: {e}"))?;
            let submit = build_submit_items(job.tenant_id, &items)?;
            let snapshot = match backend.submit(submit).await {
                Ok(snapshot) => snapshot,
                Err(e) => {
                    // The submit failed, so no provider batch was created:
                    // release the claim (`submitting → created`) with the error
                    // so the job is retried next pass rather than stranded.
                    let msg = format!("submit: {e}");
                    if let Err(release_err) = state
                        .store()
                        .release_batch_submission(job.tenant_id, job.id, &msg)
                        .await
                    {
                        tracing::error!(batch_id = %job.id, error = %release_err, "batch poll: releasing claim failed");
                    }
                    return Err(msg);
                }
            };
            // Promote `submitting → in_progress` (or, if a cancel raced in while
            // submitting, keep `canceling`, recording the provider batch id so
            // the cancel can reach the just-created provider batch).
            state
                .store()
                .mark_batch_submitted(
                    job.tenant_id,
                    job.id,
                    &snapshot.provider_batch_id,
                    snapshot.counts,
                )
                .await
                .map_err(|e| format!("mark submitted: {e}"))?;
            Ok(true)
        }
        // Claimed for submission by a pass that has not yet completed (or died
        // mid-submit). The worker never re-submits a `submitting` job — doing so
        // could double-submit under `replicas > 1` — so this is a safe no-op; the
        // owning pass finishes the transition (or ops reconciles a crash).
        BatchStatus::Submitting => Ok(false),
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
                        job.tenant_id,
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
            let Some(provider_batch_id) = job.provider_batch_id.as_deref() else {
                // Cancellation was requested while the job was `submitting` and
                // no provider batch id was ever recorded (the submit failed, or
                // the job was cancelled before it was ever submitted). Nothing
                // ran at the provider, so finalise locally as canceled.
                state
                    .store()
                    .finalize_batch_canceled(job.tenant_id, job.id)
                    .await
                    .map_err(|e| format!("finalize canceled: {e}"))?;
                return Ok(true);
            };
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
                        job.tenant_id,
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
