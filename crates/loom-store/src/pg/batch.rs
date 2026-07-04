//! [`BatchStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::model::{BatchCounts, BatchItem, BatchItemStatus, BatchJob, BatchStatus, NewBatchJob};
use crate::store::BatchStore;

/// Reconstructs a [`BatchJob`] from its stored columns.
#[allow(clippy::too_many_arguments)]
fn build_batch_job(
    id: Uuid,
    tenant_id: Uuid,
    virtual_key_id: Option<Uuid>,
    provider: String,
    status: String,
    provider_batch_id: Option<String>,
    results_url: Option<String>,
    total_items: i32,
    processing_count: i32,
    succeeded_count: i32,
    errored_count: i32,
    canceled_count: i32,
    expired_count: i32,
    error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
) -> BatchJob {
    BatchJob {
        id,
        tenant_id,
        virtual_key_id,
        provider,
        // A row Loom wrote always carries a known status; default a corrupt
        // value to `Created` rather than panicking.
        status: BatchStatus::parse(&status).unwrap_or(BatchStatus::Created),
        provider_batch_id,
        results_url,
        total_items,
        counts: BatchCounts {
            processing: processing_count,
            succeeded: succeeded_count,
            errored: errored_count,
            canceled: canceled_count,
            expired: expired_count,
        },
        error,
        created_at,
        updated_at,
        ended_at,
    }
}

/// Reconstructs a [`BatchItem`] from its stored columns.
#[allow(clippy::too_many_arguments)]
fn build_batch_item(
    id: Uuid,
    batch_id: Uuid,
    tenant_id: Uuid,
    custom_id: String,
    seq: i32,
    model: String,
    status: String,
    request: serde_json::Value,
    result: Option<serde_json::Value>,
    created_at: DateTime<Utc>,
) -> BatchItem {
    BatchItem {
        id,
        batch_id,
        tenant_id,
        custom_id,
        seq,
        model,
        status: BatchItemStatus::parse(&status),
        request,
        result,
        created_at,
    }
}

#[async_trait]
impl BatchStore for PgStore {
    async fn create_batch_job(&self, new: NewBatchJob) -> Result<BatchJob> {
        let id = Uuid::new_v4();
        let total = i32::try_from(new.items.len()).unwrap_or(i32::MAX);
        let mut tx = self.pool.begin().await?;
        let head = sqlx::query!(
            r#"
            INSERT INTO batch_jobs (id, tenant_id, virtual_key_id, provider, total_items)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING created_at, updated_at
            "#,
            id,
            new.tenant_id,
            new.virtual_key_id,
            new.provider,
            total,
        )
        .fetch_one(&mut *tx)
        .await?;

        for (index, item) in new.items.iter().enumerate() {
            let seq = i32::try_from(index).unwrap_or(i32::MAX);
            sqlx::query!(
                r#"
                INSERT INTO batch_items (
                    id, batch_id, tenant_id, custom_id, seq, model, request
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                "#,
                Uuid::new_v4(),
                id,
                new.tenant_id,
                item.custom_id,
                seq,
                item.model,
                item.request,
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(BatchJob {
            id,
            tenant_id: new.tenant_id,
            virtual_key_id: new.virtual_key_id,
            provider: new.provider,
            status: BatchStatus::Created,
            provider_batch_id: None,
            results_url: None,
            total_items: total,
            counts: BatchCounts::default(),
            error: None,
            created_at: head.created_at,
            updated_at: head.updated_at,
            ended_at: None,
        })
    }

    async fn get_batch_job(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<BatchJob>> {
        let row = sqlx::query!(
            r#"
            SELECT
                id, tenant_id, virtual_key_id, provider, status, provider_batch_id,
                results_url, total_items, processing_count, succeeded_count,
                errored_count, canceled_count, expired_count, error,
                created_at, updated_at, ended_at
            FROM batch_jobs
            WHERE id = $1 AND tenant_id = $2
            "#,
            id,
            tenant_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| {
            build_batch_job(
                r.id,
                r.tenant_id,
                r.virtual_key_id,
                r.provider,
                r.status,
                r.provider_batch_id,
                r.results_url,
                r.total_items,
                r.processing_count,
                r.succeeded_count,
                r.errored_count,
                r.canceled_count,
                r.expired_count,
                r.error,
                r.created_at,
                r.updated_at,
                r.ended_at,
            )
        }))
    }

    async fn list_batch_items(&self, tenant_id: Uuid, batch_id: Uuid) -> Result<Vec<BatchItem>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, batch_id, tenant_id, custom_id, seq, model, status, request, result, created_at
            FROM batch_items
            WHERE batch_id = $1 AND tenant_id = $2
            ORDER BY seq
            "#,
            batch_id,
            tenant_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                build_batch_item(
                    r.id,
                    r.batch_id,
                    r.tenant_id,
                    r.custom_id,
                    r.seq,
                    r.model,
                    r.status,
                    r.request,
                    r.result,
                    r.created_at,
                )
            })
            .collect())
    }

    async fn list_active_batch_jobs(&self, limit: i64) -> Result<Vec<BatchJob>> {
        let rows = sqlx::query!(
            r#"
            SELECT
                id, tenant_id, virtual_key_id, provider, status, provider_batch_id,
                results_url, total_items, processing_count, succeeded_count,
                errored_count, canceled_count, expired_count, error,
                created_at, updated_at, ended_at
            FROM batch_jobs
            WHERE status <> 'ended'
            ORDER BY created_at
            LIMIT $1
            "#,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                build_batch_job(
                    r.id,
                    r.tenant_id,
                    r.virtual_key_id,
                    r.provider,
                    r.status,
                    r.provider_batch_id,
                    r.results_url,
                    r.total_items,
                    r.processing_count,
                    r.succeeded_count,
                    r.errored_count,
                    r.canceled_count,
                    r.expired_count,
                    r.error,
                    r.created_at,
                    r.updated_at,
                    r.ended_at,
                )
            })
            .collect())
    }

    async fn get_batch_items(&self, batch_id: Uuid) -> Result<Vec<BatchItem>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, batch_id, tenant_id, custom_id, seq, model, status, request, result, created_at
            FROM batch_items
            WHERE batch_id = $1
            ORDER BY seq
            "#,
            batch_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                build_batch_item(
                    r.id,
                    r.batch_id,
                    r.tenant_id,
                    r.custom_id,
                    r.seq,
                    r.model,
                    r.status,
                    r.request,
                    r.result,
                    r.created_at,
                )
            })
            .collect())
    }

    async fn claim_batch_for_submission(&self, tenant_id: Uuid, id: Uuid) -> Result<bool> {
        // The whole point: only the caller that flips `created → submitting`
        // wins the claim. A concurrent worker (or a cancel that already moved the
        // job on) matches zero rows and must not submit.
        let result = sqlx::query!(
            r#"
            UPDATE batch_jobs
            SET status = 'submitting', updated_at = now()
            WHERE id = $1 AND tenant_id = $2 AND status = 'created'
            "#,
            id,
            tenant_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn mark_batch_submitted(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        provider_batch_id: &str,
        counts: BatchCounts,
    ) -> Result<()> {
        // Record the provider batch id and counts. Promote `submitting →
        // in_progress`, but if a cancellation raced in and moved the job to
        // `canceling`, keep it `canceling` (the worker will relay the cancel) —
        // still recording the provider batch id so the cancel can reach it. The
        // `status IN ('submitting', 'canceling')` guard means an already-ended
        // job can never be resurrected here.
        sqlx::query!(
            r#"
            UPDATE batch_jobs
            SET provider_batch_id = $3,
                status = CASE WHEN status = 'submitting' THEN 'in_progress' ELSE status END,
                processing_count = $4,
                succeeded_count = $5,
                errored_count = $6,
                canceled_count = $7,
                expired_count = $8,
                error = NULL,
                updated_at = now()
            WHERE id = $1 AND tenant_id = $2 AND status IN ('submitting', 'canceling')
            "#,
            id,
            tenant_id,
            provider_batch_id,
            counts.processing,
            counts.succeeded,
            counts.errored,
            counts.canceled,
            counts.expired,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn release_batch_submission(&self, tenant_id: Uuid, id: Uuid, error: &str) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE batch_jobs
            SET status = 'created', error = $3, updated_at = now()
            WHERE id = $1 AND tenant_id = $2 AND status = 'submitting'
            "#,
            id,
            tenant_id,
            error,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_batch_progress(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        status: BatchStatus,
        counts: BatchCounts,
        results_url: Option<&str>,
        ended_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE batch_jobs
            SET status = $3,
                processing_count = $4,
                succeeded_count = $5,
                errored_count = $6,
                canceled_count = $7,
                expired_count = $8,
                results_url = COALESCE($9, results_url),
                ended_at = COALESCE($10, ended_at),
                error = NULL,
                updated_at = now()
            WHERE id = $1 AND tenant_id = $2
            "#,
            id,
            tenant_id,
            status.as_str(),
            counts.processing,
            counts.succeeded,
            counts.errored,
            counts.canceled,
            counts.expired,
            results_url,
            ended_at,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn finalize_batch_canceled(&self, tenant_id: Uuid, id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            r#"
            UPDATE batch_jobs
            SET status = 'ended',
                canceled_count = total_items,
                processing_count = 0,
                ended_at = now(),
                updated_at = now()
            WHERE id = $1 AND tenant_id = $2 AND status <> 'ended'
            "#,
            id,
            tenant_id,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            r#"
            UPDATE batch_items
            SET status = 'canceled'
            WHERE batch_id = $1 AND tenant_id = $2 AND status = 'pending'
            "#,
            id,
            tenant_id,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn save_batch_item_result(
        &self,
        tenant_id: Uuid,
        batch_id: Uuid,
        custom_id: &str,
        status: BatchItemStatus,
        result: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE batch_items
            SET status = $4, result = $5
            WHERE batch_id = $1 AND tenant_id = $2 AND custom_id = $3
            "#,
            batch_id,
            tenant_id,
            custom_id,
            status.as_str(),
            result,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn request_batch_cancel(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<BatchJob>> {
        let mut tx = self.pool.begin().await?;
        let Some(job) = sqlx::query!(
            r#"
            SELECT status, provider_batch_id, total_items
            FROM batch_jobs
            WHERE id = $1 AND tenant_id = $2
            FOR UPDATE
            "#,
            id,
            tenant_id,
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.rollback().await?;
            return Ok(None);
        };

        let status = BatchStatus::parse(&job.status).unwrap_or(BatchStatus::Created);
        // Pick the transition by *status*, not by whether a provider batch id is
        // present: a `submitting` job (claimed by the worker, provider id not yet
        // recorded) must NOT be finalised locally, or a concurrent
        // `mark_batch_submitted` could clobber the finalisation and let the batch
        // go live. An already-`ended` job is left untouched (idempotent).
        match status {
            // Terminal — leave untouched.
            BatchStatus::Ended => {}
            // Never submitted — finalise locally, no provider round-trip.
            BatchStatus::Created => {
                sqlx::query!(
                    r#"
                    UPDATE batch_jobs
                    SET status = 'ended',
                        canceled_count = total_items,
                        processing_count = 0,
                        ended_at = now(),
                        updated_at = now()
                    WHERE id = $1
                    "#,
                    id,
                )
                .execute(&mut *tx)
                .await?;
                sqlx::query!(
                    r#"UPDATE batch_items SET status = 'canceled' WHERE batch_id = $1 AND status = 'pending'"#,
                    id,
                )
                .execute(&mut *tx)
                .await?;
            }
            // Claimed for submission, live, or already winding down: move to
            // `canceling` so the worker relays the cancellation to the provider.
            // For a `submitting` job this is the marker a racing
            // `mark_batch_submitted` observes (it then keeps `canceling` instead
            // of going to `in_progress`).
            BatchStatus::Submitting | BatchStatus::InProgress | BatchStatus::Canceling => {
                sqlx::query!(
                    r#"UPDATE batch_jobs SET status = 'canceling', updated_at = now() WHERE id = $1"#,
                    id,
                )
                .execute(&mut *tx)
                .await?;
            }
        }

        let row = sqlx::query!(
            r#"
            SELECT
                id, tenant_id, virtual_key_id, provider, status, provider_batch_id,
                results_url, total_items, processing_count, succeeded_count,
                errored_count, canceled_count, expired_count, error,
                created_at, updated_at, ended_at
            FROM batch_jobs
            WHERE id = $1
            "#,
            id,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(build_batch_job(
            row.id,
            row.tenant_id,
            row.virtual_key_id,
            row.provider,
            row.status,
            row.provider_batch_id,
            row.results_url,
            row.total_items,
            row.processing_count,
            row.succeeded_count,
            row.errored_count,
            row.canceled_count,
            row.expired_count,
            row.error,
            row.created_at,
            row.updated_at,
            row.ended_at,
        )))
    }

    async fn set_batch_error(&self, tenant_id: Uuid, id: Uuid, error: &str) -> Result<()> {
        sqlx::query!(
            r#"UPDATE batch_jobs SET error = $3, updated_at = now() WHERE id = $1 AND tenant_id = $2"#,
            id,
            tenant_id,
            error,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
