//! [`SessionStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::store::SessionStore;
use loom_core::{Session, SessionStatus};

/// Maps stored session-status text to [`SessionStatus`].
///
/// Unknown text is treated as [`SessionStatus::Active`] — fail-safe: a session
/// of unrecognised state is assumed live rather than silently dropped from a
/// conversation's lineage.
fn parse_session_status(status: &str) -> SessionStatus {
    match status {
        "superseded" => SessionStatus::Superseded,
        _ => SessionStatus::Active,
    }
}

#[async_trait]
impl SessionStore for PgStore {
    async fn get_session(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<Session>> {
        let row = sqlx::query!(
            r#"
            SELECT id, conversation_id, tenant_id, status, created_at
            FROM sessions
            WHERE id = $1 AND tenant_id = $2
            "#,
            id,
            tenant_id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| Session {
            id: row.id,
            conversation_id: row.conversation_id,
            tenant_id: row.tenant_id,
            status: parse_session_status(&row.status),
            created_at: row.created_at,
        }))
    }

    async fn list_sessions(&self, tenant_id: Uuid, conversation_id: Uuid) -> Result<Vec<Session>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, conversation_id, tenant_id, status, created_at
            FROM sessions
            WHERE conversation_id = $1 AND tenant_id = $2
            ORDER BY created_at, id
            "#,
            conversation_id,
            tenant_id,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| Session {
                id: row.id,
                conversation_id: row.conversation_id,
                tenant_id: row.tenant_id,
                status: parse_session_status(&row.status),
                created_at: row.created_at,
            })
            .collect())
    }
}
