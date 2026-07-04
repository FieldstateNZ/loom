//! PostgreSQL implementation of the store traits.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

use crate::error::Result;
use crate::model::{
    KeyBudget, NewProviderCredential, NewTenant, NewUsageEvent, NewVirtualKey, ProviderCredential,
    Tenant, UsageEvent, UsageRollup, VirtualKey,
};
use crate::store::{ConversationStore, CredentialStore, KeyStore, TenantStore, UsageStore};
use loom_core::{ContentPart, Conversation, Message, ProviderBinding, Role, Usage};

/// A PostgreSQL-backed store implementing every store trait over a shared
/// connection pool.
///
/// Clone is cheap — the pool is reference-counted — so a single `PgStore` can
/// be shared across request handlers.
#[derive(Clone, Debug)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    /// Wraps an existing connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Connects to the database at `url`, building a default pool.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if a connection cannot be established.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new().connect(url).await?;
        Ok(Self::new(pool))
    }

    /// Borrows the underlying connection pool.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

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

/// Reconstructs a [`VirtualKey`] from its stored columns.
#[allow(clippy::too_many_arguments)]
fn build_virtual_key(
    id: Uuid,
    tenant_id: Uuid,
    key_hash: String,
    key_prefix: String,
    name: String,
    status: String,
    scopes: serde_json::Value,
    budget_limit_amount: Option<Decimal>,
    budget_window: Option<String>,
    budget_action: Option<String>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
) -> Result<VirtualKey> {
    let scopes: Vec<String> = serde_json::from_value(scopes)?;
    let budget = budget_limit_amount.map(|limit_amount| KeyBudget {
        limit_amount,
        window: budget_window.unwrap_or_default(),
        action: budget_action.unwrap_or_default(),
    });
    Ok(VirtualKey {
        id,
        tenant_id,
        key_hash,
        key_prefix,
        name,
        status,
        scopes,
        budget,
        created_at,
        last_used_at,
    })
}

#[async_trait]
impl TenantStore for PgStore {
    async fn create_tenant(&self, new: NewTenant) -> Result<Tenant> {
        let id = Uuid::new_v4();
        let row = sqlx::query!(
            r#"
            INSERT INTO tenants (id, slug, name, status)
            VALUES ($1, $2, $3, $4)
            RETURNING id, slug, name, status, created_at
            "#,
            id,
            new.slug,
            new.name,
            new.status,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(Tenant {
            id: row.id,
            slug: row.slug,
            name: row.name,
            status: row.status,
            created_at: row.created_at,
        })
    }

    async fn get_tenant(&self, id: Uuid) -> Result<Option<Tenant>> {
        let row = sqlx::query!(
            r#"
            SELECT id, slug, name, status, created_at
            FROM tenants
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| Tenant {
            id: row.id,
            slug: row.slug,
            name: row.name,
            status: row.status,
            created_at: row.created_at,
        }))
    }

    async fn get_tenant_by_slug(&self, slug: &str) -> Result<Option<Tenant>> {
        let row = sqlx::query!(
            r#"
            SELECT id, slug, name, status, created_at
            FROM tenants
            WHERE slug = $1
            "#,
            slug,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| Tenant {
            id: row.id,
            slug: row.slug,
            name: row.name,
            status: row.status,
            created_at: row.created_at,
        }))
    }
}

#[async_trait]
impl KeyStore for PgStore {
    async fn create_key(&self, new: NewVirtualKey) -> Result<VirtualKey> {
        let id = Uuid::new_v4();
        let scopes = serde_json::to_value(&new.scopes)?;
        let (limit_amount, window, action) = match new.budget {
            Some(b) => (Some(b.limit_amount), Some(b.window), Some(b.action)),
            None => (None, None, None),
        };
        let row = sqlx::query!(
            r#"
            INSERT INTO virtual_keys (
                id, tenant_id, key_hash, key_prefix, name, scopes,
                budget_limit_amount, budget_window, budget_action
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING
                id, tenant_id, key_hash, key_prefix, name, status, scopes,
                budget_limit_amount, budget_window, budget_action,
                created_at, last_used_at
            "#,
            id,
            new.tenant_id,
            new.key_hash,
            new.key_prefix,
            new.name,
            scopes,
            limit_amount,
            window,
            action,
        )
        .fetch_one(&self.pool)
        .await?;
        build_virtual_key(
            row.id,
            row.tenant_id,
            row.key_hash,
            row.key_prefix,
            row.name,
            row.status,
            row.scopes,
            row.budget_limit_amount,
            row.budget_window,
            row.budget_action,
            row.created_at,
            row.last_used_at,
        )
    }

    async fn get_key_by_hash(&self, key_hash: &str) -> Result<Option<VirtualKey>> {
        let row = sqlx::query!(
            r#"
            SELECT
                id, tenant_id, key_hash, key_prefix, name, status, scopes,
                budget_limit_amount, budget_window, budget_action,
                created_at, last_used_at
            FROM virtual_keys
            WHERE key_hash = $1
            "#,
            key_hash,
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            build_virtual_key(
                row.id,
                row.tenant_id,
                row.key_hash,
                row.key_prefix,
                row.name,
                row.status,
                row.scopes,
                row.budget_limit_amount,
                row.budget_window,
                row.budget_action,
                row.created_at,
                row.last_used_at,
            )
        })
        .transpose()
    }

    async fn revoke_key(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            UPDATE virtual_keys
            SET status = 'revoked'
            WHERE id = $1
            "#,
            id,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn touch_key_last_used(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            UPDATE virtual_keys
            SET last_used_at = now()
            WHERE id = $1
            "#,
            id,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[async_trait]
impl CredentialStore for PgStore {
    async fn upsert_credential(&self, new: NewProviderCredential) -> Result<ProviderCredential> {
        let id = Uuid::new_v4();
        let row = sqlx::query!(
            r#"
            INSERT INTO provider_credentials (
                id, tenant_id, provider, encrypted_secret, nonce, aad, base_url
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT ON CONSTRAINT uq_provider_credentials_tenant_provider
            DO UPDATE SET
                encrypted_secret = EXCLUDED.encrypted_secret,
                nonce = EXCLUDED.nonce,
                aad = EXCLUDED.aad,
                base_url = EXCLUDED.base_url
            RETURNING
                id, tenant_id, provider, encrypted_secret, nonce, aad,
                base_url, created_at
            "#,
            id,
            new.tenant_id,
            new.provider,
            new.encrypted_secret,
            new.nonce,
            new.aad,
            new.base_url,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(ProviderCredential {
            id: row.id,
            tenant_id: row.tenant_id,
            provider: row.provider,
            encrypted_secret: row.encrypted_secret,
            nonce: row.nonce,
            aad: row.aad,
            base_url: row.base_url,
            created_at: row.created_at,
        })
    }

    async fn get_credential(
        &self,
        tenant_id: Option<Uuid>,
        provider: &str,
    ) -> Result<Option<ProviderCredential>> {
        let row = sqlx::query!(
            r#"
            SELECT
                id, tenant_id, provider, encrypted_secret, nonce, aad,
                base_url, created_at
            FROM provider_credentials
            WHERE tenant_id IS NOT DISTINCT FROM $1 AND provider = $2
            "#,
            tenant_id,
            provider,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| ProviderCredential {
            id: row.id,
            tenant_id: row.tenant_id,
            provider: row.provider,
            encrypted_secret: row.encrypted_secret,
            nonce: row.nonce,
            aad: row.aad,
            base_url: row.base_url,
            created_at: row.created_at,
        }))
    }

    async fn list_credentials(&self, tenant_id: Option<Uuid>) -> Result<Vec<ProviderCredential>> {
        let rows = sqlx::query!(
            r#"
            SELECT
                id, tenant_id, provider, encrypted_secret, nonce, aad,
                base_url, created_at
            FROM provider_credentials
            WHERE tenant_id IS NOT DISTINCT FROM $1
            ORDER BY provider
            "#,
            tenant_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| ProviderCredential {
                id: row.id,
                tenant_id: row.tenant_id,
                provider: row.provider,
                encrypted_secret: row.encrypted_secret,
                nonce: row.nonce,
                aad: row.aad,
                base_url: row.base_url,
                created_at: row.created_at,
            })
            .collect())
    }
}

#[async_trait]
impl ConversationStore for PgStore {
    async fn create_conversation(&self, conversation: &Conversation) -> Result<()> {
        let mut tx = self.pool.begin().await?;
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
                    id, conversation_id, seq, role, content, raw_provider_payload, usage
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                "#,
                Uuid::new_v4(),
                conversation.id,
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
                created_at, updated_at
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

        Ok(Some(Conversation {
            id: head.id,
            tenant_id: head.tenant_id,
            binding: ProviderBinding::new(head.provider, head.model),
            system: head.system,
            messages,
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
                id, conversation_id, seq, role, content, raw_provider_payload, usage
            )
            VALUES (
                $1,
                $2,
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

#[async_trait]
impl UsageStore for PgStore {
    async fn record_event(&self, event: NewUsageEvent) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let usage = &event.usage;
        let input_tokens = usage
            .input_tokens
            .map_or(0, |v| i64::try_from(v).unwrap_or(i64::MAX));
        let output_tokens = usage
            .output_tokens
            .map_or(0, |v| i64::try_from(v).unwrap_or(i64::MAX));
        let cache_read_tokens = usage
            .cache_read_tokens
            .map_or(0, |v| i64::try_from(v).unwrap_or(i64::MAX));
        let cache_write_tokens = usage
            .cache_write_tokens
            .map_or(0, |v| i64::try_from(v).unwrap_or(i64::MAX));
        let server_tool_counts = serde_json::to_value(&usage.server_tool_use)?;
        let raw_usage = usage.raw.clone();

        let row = sqlx::query!(
            r#"
            INSERT INTO usage_events (
                id, tenant_id, virtual_key_id, conversation_id, provider, model,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                server_tool_counts, cost, raw_usage
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            RETURNING id
            "#,
            id,
            event.tenant_id,
            event.virtual_key_id,
            event.conversation_id,
            event.provider,
            event.model,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            server_tool_counts,
            event.cost,
            raw_usage,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.id)
    }

    async fn list_events(&self, tenant_id: Uuid, limit: i64) -> Result<Vec<UsageEvent>> {
        let rows = sqlx::query!(
            r#"
            SELECT
                id, tenant_id, virtual_key_id, conversation_id, provider, model,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                server_tool_counts, cost, raw_usage, created_at
            FROM usage_events
            WHERE tenant_id = $1
            ORDER BY created_at DESC, id DESC
            LIMIT $2
            "#,
            tenant_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| UsageEvent {
                id: row.id,
                tenant_id: row.tenant_id,
                virtual_key_id: row.virtual_key_id,
                conversation_id: row.conversation_id,
                provider: row.provider,
                model: row.model,
                input_tokens: row.input_tokens,
                output_tokens: row.output_tokens,
                cache_read_tokens: row.cache_read_tokens,
                cache_write_tokens: row.cache_write_tokens,
                server_tool_counts: row.server_tool_counts,
                cost: row.cost,
                raw_usage: row.raw_usage,
                created_at: row.created_at,
            })
            .collect())
    }

    async fn rollup(&self, tenant_id: Uuid) -> Result<UsageRollup> {
        let row = sqlx::query!(
            r#"
            SELECT
                COUNT(*) AS "event_count!",
                COALESCE(SUM(input_tokens), 0)::bigint AS "input_tokens!",
                COALESCE(SUM(output_tokens), 0)::bigint AS "output_tokens!",
                COALESCE(SUM(cache_read_tokens), 0)::bigint AS "cache_read_tokens!",
                COALESCE(SUM(cache_write_tokens), 0)::bigint AS "cache_write_tokens!"
            FROM usage_events
            WHERE tenant_id = $1
            "#,
            tenant_id,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(UsageRollup {
            event_count: row.event_count,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cache_read_tokens: row.cache_read_tokens,
            cache_write_tokens: row.cache_write_tokens,
        })
    }
}
