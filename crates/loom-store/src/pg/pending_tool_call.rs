//! [`PendingToolCallStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::model::NewPendingToolCall;
use crate::store::{PendingToolCallStore, ResolveOutcome};
use loom_core::{PendingToolCall, ToolKind};

/// Maps stored tool-kind text to [`ToolKind`]. Unknown text is treated as
/// [`ToolKind::Custom`] — the stricter side of `drain`'s builtin carve-out.
fn parse_tool_kind(kind: &str) -> ToolKind {
    match kind {
        "builtin" => ToolKind::Builtin,
        _ => ToolKind::Custom,
    }
}

/// The stored text form of a [`ToolKind`].
fn tool_kind_text(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Custom => "custom",
        ToolKind::Builtin => "builtin",
    }
}

#[async_trait]
impl PendingToolCallStore for PgStore {
    async fn record_pending(
        &self,
        tenant_id: Uuid,
        session_id: Uuid,
        call: NewPendingToolCall,
    ) -> Result<Option<PendingToolCall>> {
        let id = Uuid::new_v4();
        let kind_text = tool_kind_text(call.kind);

        let mut tx = self.pool.begin().await?;
        // Only record against a session this tenant owns; a missing or foreign
        // session no-ops.
        let owned = sqlx::query!(
            r#"SELECT id FROM sessions WHERE id = $1 AND tenant_id = $2 FOR UPDATE"#,
            session_id,
            tenant_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if owned.is_none() {
            tx.rollback().await?;
            return Ok(None);
        }

        let row = sqlx::query!(
            r#"
            INSERT INTO pending_tool_calls (
                id, session_id, tool_use_id, kind, name, input, mcp_server_url
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING created_at
            "#,
            id,
            session_id,
            call.tool_use_id,
            kind_text,
            call.name,
            call.input,
            call.mcp_server_url,
        )
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(PendingToolCall {
            id,
            session_id,
            tool_use_id: call.tool_use_id,
            kind: call.kind,
            name: call.name,
            input: call.input,
            mcp_server_url: call.mcp_server_url,
            created_at: row.created_at,
        }))
    }

    async fn list_pending(
        &self,
        tenant_id: Uuid,
        session_id: Uuid,
    ) -> Result<Vec<PendingToolCall>> {
        let rows = sqlx::query!(
            r#"
            SELECT p.id, p.session_id, p.tool_use_id, p.kind, p.name,
                   p.input, p.mcp_server_url, p.created_at
            FROM pending_tool_calls p
            JOIN sessions s ON s.id = p.session_id
            WHERE p.session_id = $1 AND s.tenant_id = $2 AND p.resolved_at IS NULL
            ORDER BY p.created_at, p.id
            "#,
            session_id,
            tenant_id,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| PendingToolCall {
                id: row.id,
                session_id: row.session_id,
                tool_use_id: row.tool_use_id,
                kind: parse_tool_kind(&row.kind),
                name: row.name,
                input: row.input,
                mcp_server_url: row.mcp_server_url,
                created_at: row.created_at,
            })
            .collect())
    }

    async fn resolve(
        &self,
        tenant_id: Uuid,
        session_id: Uuid,
        tool_use_id: &str,
        result: serde_json::Value,
        is_error: bool,
    ) -> Result<ResolveOutcome> {
        let mut tx = self.pool.begin().await?;
        // Lock the matching call (tenant-scoped via its session) so the
        // resolved/unresolved decision and the update are atomic.
        let row = sqlx::query!(
            r#"
            SELECT p.id, (p.resolved_at IS NOT NULL) AS "resolved!"
            FROM pending_tool_calls p
            JOIN sessions s ON s.id = p.session_id
            WHERE p.session_id = $1 AND p.tool_use_id = $2 AND s.tenant_id = $3
            FOR UPDATE OF p
            "#,
            session_id,
            tool_use_id,
            tenant_id,
        )
        .fetch_optional(&mut *tx)
        .await?;

        let outcome = match row {
            None => ResolveOutcome::NotFound,
            Some(row) if row.resolved => ResolveOutcome::AlreadyResolved,
            Some(row) => {
                sqlx::query!(
                    r#"
                    UPDATE pending_tool_calls
                    SET resolved_at = now(), result = $2, is_error = $3
                    WHERE id = $1
                    "#,
                    row.id,
                    result,
                    is_error,
                )
                .execute(&mut *tx)
                .await?;
                ResolveOutcome::Resolved
            }
        };

        tx.commit().await?;
        Ok(outcome)
    }
}
