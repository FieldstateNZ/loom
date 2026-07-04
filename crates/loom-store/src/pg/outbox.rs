//! [`OutboxStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::model::{NewUsageEvent, OutboxEntry};
use crate::store::OutboxStore;

#[async_trait]
impl OutboxStore for PgStore {
    async fn enqueue_outbox(&self, event: &NewUsageEvent) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let payload = serde_json::to_value(event)?;
        sqlx::query!(
            r#"
            INSERT INTO usage_outbox (id, payload)
            VALUES ($1, $2)
            "#,
            id,
            payload,
        )
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    async fn list_pending_outbox(&self, limit: i64) -> Result<Vec<OutboxEntry>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, payload, status, attempts, last_error, created_at
            FROM usage_outbox
            WHERE status = 'pending'
            ORDER BY created_at
            LIMIT $1
            "#,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            entries.push(OutboxEntry {
                id: row.id,
                payload: serde_json::from_value(row.payload)?,
                status: row.status,
                attempts: row.attempts,
                last_error: row.last_error,
                created_at: row.created_at,
            });
        }
        Ok(entries)
    }

    async fn mark_outbox_processed(&self, id: Uuid) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE usage_outbox
            SET status = 'processed', processed_at = now()
            WHERE id = $1
            "#,
            id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_outbox_failed(&self, id: Uuid, error: &str) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE usage_outbox
            SET attempts = attempts + 1, last_error = $2
            WHERE id = $1
            "#,
            id,
            error,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
