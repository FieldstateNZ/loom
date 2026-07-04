//! Integration tests for asynchronous batch job persistence and lifecycle
//! transitions, against a real database.

mod common;

use chrono::Utc;
use serde_json::json;

use loom_store::{
    BatchCounts, BatchItemStatus, BatchStatus, BatchStore, NewBatchItem, NewBatchJob, NewTenant,
    TenantStore,
};

/// The batch store round-trips a job through its lifecycle: create with items,
/// submit, persist per-item results, finalise, and read back — all tenant-scoped.
#[tokio::test]
async fn batch_job_lifecycle_and_tenant_scope() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("batch", "Batch Tenant"))
        .await
        .unwrap();
    let other = store
        .create_tenant(NewTenant::new("other-batch", "Other"))
        .await
        .unwrap();

    let job = store
        .create_batch_job(NewBatchJob {
            tenant_id: tenant.id,
            virtual_key_id: None,
            provider: "anthropic".to_owned(),
            items: vec![
                NewBatchItem {
                    custom_id: "a".to_owned(),
                    model: "claude-opus-4-8".to_owned(),
                    request: json!({ "provider": "anthropic", "model": "claude-opus-4-8" }),
                },
                NewBatchItem {
                    custom_id: "b".to_owned(),
                    model: "claude-opus-4-8".to_owned(),
                    request: json!({ "provider": "anthropic", "model": "claude-opus-4-8" }),
                },
            ],
        })
        .await
        .unwrap();
    assert_eq!(job.status, BatchStatus::Created);
    assert_eq!(job.total_items, 2);

    // Items land pending, in submission order.
    let items = store.get_batch_items(job.id).await.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].custom_id, "a");
    assert_eq!(items[0].status, BatchItemStatus::Pending);

    // The worker sees it as active until it ends.
    let active = store.list_active_batch_jobs(10).await.unwrap();
    assert_eq!(active.len(), 1);

    // Claim (created → submitting) then submit (submitting → in_progress). The
    // claim is exactly-once: a second attempt on the same job wins no row.
    assert!(store
        .claim_batch_for_submission(tenant.id, job.id)
        .await
        .unwrap());
    assert!(
        !store
            .claim_batch_for_submission(tenant.id, job.id)
            .await
            .unwrap(),
        "a job already claimed for submission cannot be claimed again"
    );
    store
        .mark_batch_submitted(
            tenant.id,
            job.id,
            "msgbatch_1",
            BatchCounts {
                processing: 2,
                ..BatchCounts::default()
            },
        )
        .await
        .unwrap();
    let submitted = store
        .get_batch_job(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(submitted.status, BatchStatus::InProgress);
    assert_eq!(submitted.provider_batch_id.as_deref(), Some("msgbatch_1"));
    assert_eq!(submitted.counts.processing, 2);

    // Persist results and finalise.
    for cid in ["a", "b"] {
        store
            .save_batch_item_result(
                tenant.id,
                job.id,
                cid,
                BatchItemStatus::Succeeded,
                &json!({ "type": "succeeded" }),
            )
            .await
            .unwrap();
    }
    store
        .update_batch_progress(
            tenant.id,
            job.id,
            BatchStatus::Ended,
            BatchCounts {
                succeeded: 2,
                ..BatchCounts::default()
            },
            Some("https://example/results"),
            Some(Utc::now()),
        )
        .await
        .unwrap();

    let ended = store
        .get_batch_job(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ended.status, BatchStatus::Ended);
    assert_eq!(ended.counts.succeeded, 2);
    assert!(ended.ended_at.is_some());
    assert!(store.list_active_batch_jobs(10).await.unwrap().is_empty());

    // Tenant-scoped reads: a foreign tenant sees nothing.
    let scoped = store.list_batch_items(tenant.id, job.id).await.unwrap();
    assert_eq!(scoped.len(), 2);
    assert!(scoped.iter().all(|i| i.result.is_some()));
    assert!(store
        .get_batch_job(other.id, job.id)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .list_batch_items(other.id, job.id)
        .await
        .unwrap()
        .is_empty());
}

/// Cancelling a job that was never submitted finalises it immediately, marking
/// every item canceled without any provider round-trip.
#[tokio::test]
async fn batch_cancel_before_submission_finalises_locally() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("precancel", "Pre-cancel"))
        .await
        .unwrap();
    let job = store
        .create_batch_job(NewBatchJob {
            tenant_id: tenant.id,
            virtual_key_id: None,
            provider: "anthropic".to_owned(),
            items: vec![NewBatchItem {
                custom_id: "only".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                request: json!({ "provider": "anthropic", "model": "claude-opus-4-8" }),
            }],
        })
        .await
        .unwrap();

    let canceled = store
        .request_batch_cancel(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(canceled.status, BatchStatus::Ended);
    assert_eq!(canceled.counts.canceled, 1);
    assert!(canceled.ended_at.is_some());

    let items = store.get_batch_items(job.id).await.unwrap();
    assert_eq!(items[0].status, BatchItemStatus::Canceled);

    // A foreign tenant cannot cancel it.
    let intruder = store
        .create_tenant(NewTenant::new("intruder-b", "Intruder"))
        .await
        .unwrap();
    assert!(store
        .request_batch_cancel(intruder.id, job.id)
        .await
        .unwrap()
        .is_none());
}

/// A cancellation that arrives while a job is being submitted must not be
/// clobbered by the submit completing: the job stays `canceling` (never goes
/// live) and, once the provider batch id is recorded, the worker can relay the
/// cancel. This is the store-level guarantee behind finding #2.
#[tokio::test]
async fn cancel_during_submitting_is_not_resurrected() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("cancel-race", "Cancel Race"))
        .await
        .unwrap();
    let job = store
        .create_batch_job(NewBatchJob {
            tenant_id: tenant.id,
            virtual_key_id: None,
            provider: "anthropic".to_owned(),
            items: vec![NewBatchItem {
                custom_id: "only".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                request: json!({ "provider": "anthropic", "model": "claude-opus-4-8" }),
            }],
        })
        .await
        .unwrap();

    // Worker claims the job for submission (created → submitting).
    assert!(store
        .claim_batch_for_submission(tenant.id, job.id)
        .await
        .unwrap());

    // A cancel arrives *during* submission: it must move the job to `canceling`,
    // NOT finalise it locally (which a concurrent submit could clobber).
    let canceling = store
        .request_batch_cancel(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(canceling.status, BatchStatus::Canceling);

    // The submit now completes. `mark_batch_submitted` must keep the job
    // `canceling` (the guarded CASE) while still recording the provider batch id
    // so the cancellation can reach the live provider batch.
    store
        .mark_batch_submitted(
            tenant.id,
            job.id,
            "msgbatch_race",
            BatchCounts {
                processing: 1,
                ..BatchCounts::default()
            },
        )
        .await
        .unwrap();
    let after = store
        .get_batch_job(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.status,
        BatchStatus::Canceling,
        "a cancel during submitting must not be flipped back to in_progress"
    );
    assert_eq!(after.provider_batch_id.as_deref(), Some("msgbatch_race"));
}

/// Releasing a claim after a failed submission reverts `submitting → created`
/// (with the error recorded) so the job is retried, and never touches a job that
/// a cancellation has already moved on.
#[tokio::test]
async fn release_batch_submission_reverts_to_created() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("release", "Release"))
        .await
        .unwrap();
    let job = store
        .create_batch_job(NewBatchJob {
            tenant_id: tenant.id,
            virtual_key_id: None,
            provider: "anthropic".to_owned(),
            items: vec![NewBatchItem {
                custom_id: "only".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                request: json!({ "provider": "anthropic", "model": "claude-opus-4-8" }),
            }],
        })
        .await
        .unwrap();

    assert!(store
        .claim_batch_for_submission(tenant.id, job.id)
        .await
        .unwrap());
    store
        .release_batch_submission(tenant.id, job.id, "submit: upstream 503")
        .await
        .unwrap();
    let reverted = store
        .get_batch_job(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reverted.status, BatchStatus::Created);
    assert_eq!(reverted.error.as_deref(), Some("submit: upstream 503"));

    // It can be claimed again (retry).
    assert!(store
        .claim_batch_for_submission(tenant.id, job.id)
        .await
        .unwrap());
}
