//! [`ConversationStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::store::ConversationStore;
use loom_core::{ContentPart, Conversation, Message, ProviderBinding, Role, Usage};

/// Serialises a [`Role`] to its stored text form.
fn role_to_text(role: Role) -> String {
    match serde_json::to_value(role) {
        // `Role` is a unit-variant enum, so it always serialises to a JSON
        // string; the fallback keeps future `#[non_exhaustive]` roles working.
        Ok(serde_json::Value::String(s)) => s,
        _ => unreachable!("Role always serialises to a JSON string"),
    }
}

/// Reconstructs a [`Message`] from its stored columns.
fn build_message(
    role: String,
    content: serde_json::Value,
    usage: Option<serde_json::Value>,
    raw: Option<serde_json::Value>,
) -> Result<Message> {
    let role: Role = serde_json::from_value(serde_json::Value::String(role))?;
    let content: Vec<ContentPart> = serde_json::from_value(content)?;
    let usage: Option<Usage> = usage.map(serde_json::from_value).transpose()?;
    Ok(Message {
        role,
        content,
        usage,
        // The verbatim provider payload is preserved from
        // `messages.raw_provider_payload` so history round-trips losslessly.
        raw,
    })
}

#[async_trait]
impl ConversationStore for PgStore {
    async fn create_conversation(&self, conversation: &Conversation) -> Result<()> {
        // The conversation's active session id. `Conversation::new` mints one;
        // fall back to a fresh id for a hand-built conversation without one.
        let session_id = conversation.current_session_id.unwrap_or_else(Uuid::new_v4);

        let mut tx = self.pool.begin().await?;
        // The conversation and its session reference each other, so insert the
        // conversation first (current_session_id left NULL), then its session,
        // then link them.
        sqlx::query!(
            r#"
            INSERT INTO conversations (
                id, tenant_id, provider, model, system, metadata,
                created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            conversation.id,
            conversation.tenant_id,
            conversation.binding.provider,
            conversation.binding.model,
            conversation.system,
            conversation.metadata,
            conversation.created_at,
            conversation.updated_at,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO sessions (id, conversation_id, tenant_id, status, created_at)
            VALUES ($1, $2, $3, 'active', $4)
            "#,
            session_id,
            conversation.id,
            conversation.tenant_id,
            conversation.created_at,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            r#"UPDATE conversations SET current_session_id = $1 WHERE id = $2"#,
            session_id,
            conversation.id,
        )
        .execute(&mut *tx)
        .await?;

        for (index, message) in conversation.messages.iter().enumerate() {
            let seq = i32::try_from(index).unwrap_or(i32::MAX);
            let role = role_to_text(message.role);
            let content = serde_json::to_value(&message.content)?;
            let usage = message
                .usage
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?;
            let raw = message.raw.clone();
            sqlx::query!(
                r#"
                INSERT INTO messages (
                    id, conversation_id, session_id, seq, role,
                    content, raw_provider_payload, usage
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                "#,
                Uuid::new_v4(),
                conversation.id,
                session_id,
                seq,
                role,
                content,
                raw,
                usage,
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn get_conversation(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<Conversation>> {
        let Some(head) = sqlx::query!(
            r#"
            SELECT
                id, tenant_id, provider, model, system, metadata,
                current_session_id, created_at, updated_at
            FROM conversations
            WHERE id = $1 AND tenant_id = $2
            "#,
            id,
            tenant_id,
        )
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };

        let rows = sqlx::query!(
            r#"
            SELECT role, content, usage, raw_provider_payload
            FROM messages
            WHERE conversation_id = $1
            ORDER BY seq
            "#,
            id,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            messages.push(build_message(
                row.role,
                row.content,
                row.usage,
                row.raw_provider_payload,
            )?);
        }

        // Lineage: the conversation's superseded sessions — every session bar
        // the current one — oldest-first. Empty until the conversation has
        // migrated at least once.
        let previous_session_ids = sqlx::query_scalar!(
            r#"
            SELECT id
            FROM sessions
            WHERE conversation_id = $1
              AND ($2::uuid IS NULL OR id <> $2)
            ORDER BY created_at, id
            "#,
            id,
            head.current_session_id,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(Some(Conversation {
            id: head.id,
            tenant_id: head.tenant_id,
            binding: ProviderBinding::new(head.provider, head.model),
            system: head.system,
            // The system-prompt cache hint is a request-render concern, not
            // durable state; it is not persisted and reconstructs as unset.
            system_cache: None,
            messages,
            current_session_id: head.current_session_id,
            previous_session_ids,
            metadata: head.metadata,
            created_at: head.created_at,
            updated_at: head.updated_at,
        }))
    }

    async fn list_messages(
        &self,
        tenant_id: Uuid,
        conversation_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Message>> {
        let rows = sqlx::query!(
            r#"
            SELECT role, content, usage, raw_provider_payload
            FROM messages
            WHERE conversation_id = $1
              AND EXISTS (
                  SELECT 1 FROM conversations c
                  WHERE c.id = $1 AND c.tenant_id = $2
              )
            ORDER BY seq
            LIMIT $3 OFFSET $4
            "#,
            conversation_id,
            tenant_id,
            limit,
            offset,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            messages.push(build_message(
                row.role,
                row.content,
                row.usage,
                row.raw_provider_payload,
            )?);
        }
        Ok(messages)
    }

    async fn append_message(
        &self,
        tenant_id: Uuid,
        conversation_id: Uuid,
        message: &Message,
    ) -> Result<Option<i32>> {
        let role = role_to_text(message.role);
        let content = serde_json::to_value(&message.content)?;
        let usage = message
            .usage
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        let raw = message.raw.clone();

        let mut tx = self.pool.begin().await?;
        // Lock the conversation row, but only if it belongs to the tenant. A
        // conversation owned by another tenant (or a missing one) yields no
        // row, so the append no-ops without touching another tenant's history.
        let owned = sqlx::query!(
            r#"SELECT id FROM conversations WHERE id = $1 AND tenant_id = $2 FOR UPDATE"#,
            conversation_id,
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
            INSERT INTO messages (
                id, conversation_id, session_id, seq, role,
                content, raw_provider_payload, usage
            )
            VALUES (
                $1,
                $2,
                (SELECT current_session_id FROM conversations WHERE id = $2),
                (SELECT COALESCE(MAX(seq), -1) + 1 FROM messages WHERE conversation_id = $2),
                $3,
                $4,
                $5,
                $6
            )
            RETURNING seq
            "#,
            Uuid::new_v4(),
            conversation_id,
            role,
            content,
            raw,
            usage,
        )
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query!(
            r#"UPDATE conversations SET updated_at = now() WHERE id = $1"#,
            conversation_id,
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(row.seq))
    }

    async fn delete_conversation(&self, tenant_id: Uuid, id: Uuid) -> Result<bool> {
        let result = sqlx::query!(
            r#"DELETE FROM conversations WHERE id = $1 AND tenant_id = $2"#,
            id,
            tenant_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
