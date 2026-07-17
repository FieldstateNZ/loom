//! [`SessionEventStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::store::SessionEventStore;
use loom_core::{Event, EventKind};

/// Formats a per-session sequence as the event's opaque, lexicographically
/// monotonic id — zero-padded so string ordering matches numeric ordering
/// (`"…010"` sorts after `"…009"`).
fn event_id(seq: i64) -> String {
    format!("{seq:020}")
}

/// Parses an event-id cursor back to its sequence. A missing or malformed cursor
/// resolves to `-1` ("before the first event"), so the page starts from the
/// beginning rather than erroring.
fn cursor_seq(after: Option<&str>) -> i64 {
    after.and_then(|s| s.parse::<i64>().ok()).unwrap_or(-1)
}

#[async_trait]
impl SessionEventStore for PgStore {
    async fn append_event(
        &self,
        tenant_id: Uuid,
        session_id: Uuid,
        kind: &EventKind,
    ) -> Result<Option<Event>> {
        let kind_json = serde_json::to_value(kind)?;

        let mut tx = self.pool.begin().await?;
        // Lock the session (only if it belongs to the tenant) so per-session seq
        // assignment serialises; a missing or foreign session no-ops.
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
            INSERT INTO session_events (session_id, seq, kind)
            VALUES (
                $1,
                (SELECT COALESCE(MAX(seq), -1) + 1 FROM session_events WHERE session_id = $1),
                $2
            )
            RETURNING seq, at
            "#,
            session_id,
            kind_json,
        )
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(Event {
            id: event_id(row.seq),
            at: row.at,
            kind: kind.clone(),
        }))
    }

    async fn list_session_events(
        &self,
        tenant_id: Uuid,
        session_id: Uuid,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Event>> {
        let after_seq = cursor_seq(after);
        let rows = sqlx::query!(
            r#"
            SELECT e.seq, e.kind, e.at
            FROM session_events e
            JOIN sessions s ON s.id = e.session_id
            WHERE e.session_id = $1 AND s.tenant_id = $2 AND e.seq > $3
            ORDER BY e.seq
            LIMIT $4
            "#,
            session_id,
            tenant_id,
            after_seq,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            events.push(Event {
                id: event_id(row.seq),
                at: row.at,
                kind: serde_json::from_value(row.kind)?,
            });
        }
        Ok(events)
    }
}
