//! PostgreSQL implementation of the store traits.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

use crate::error::Result;
use crate::model::{
    Budget, BudgetAction, BudgetWindow, ModelPrice, NewModelPrice, NewProviderCredential,
    NewTenant, NewUsageEvent, NewVirtualKey, OutboxEntry, ProviderCredential, RateLimit,
    RollupGroup, Tenant, UsageEvent, UsageRollup, UsageRollupRow, VirtualKey,
};
use crate::store::{
    BudgetStore, ConversationStore, CredentialStore, KeyStore, OutboxStore, PricingStore,
    TenantStore, UsageStore,
};
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

/// Assembles a [`Budget`] from its three nullable stored columns, treating a
/// present `limit_amount` as the presence signal. Unknown window/action text
/// falls back to the safe defaults (`Total` window, `Block` action) so a corrupt
/// row never silently disables enforcement.
fn build_budget(
    limit_amount: Option<Decimal>,
    window: Option<String>,
    action: Option<String>,
) -> Option<Budget> {
    limit_amount.map(|limit_amount| Budget {
        limit_amount,
        window: window
            .as_deref()
            .and_then(BudgetWindow::parse)
            .unwrap_or(BudgetWindow::Total),
        action: action
            .as_deref()
            .and_then(BudgetAction::parse)
            .unwrap_or(BudgetAction::Block),
    })
}

/// Assembles a [`RateLimit`] from its two nullable stored columns, or `None`
/// when neither dimension is set.
fn build_rate_limit(
    requests_per_min: Option<i64>,
    tokens_per_min: Option<i64>,
) -> Option<RateLimit> {
    if requests_per_min.is_none() && tokens_per_min.is_none() {
        return None;
    }
    Some(RateLimit {
        requests_per_min,
        tokens_per_min,
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
    rate_limit_requests_per_min: Option<i64>,
    rate_limit_tokens_per_min: Option<i64>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
) -> Result<VirtualKey> {
    let scopes: Vec<String> = serde_json::from_value(scopes)?;
    Ok(VirtualKey {
        id,
        tenant_id,
        key_hash,
        key_prefix,
        name,
        status,
        scopes,
        budget: build_budget(budget_limit_amount, budget_window, budget_action),
        rate_limit: build_rate_limit(rate_limit_requests_per_min, rate_limit_tokens_per_min),
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
            Some(b) => (
                Some(b.limit_amount),
                Some(b.window.as_str().to_owned()),
                Some(b.action.as_str().to_owned()),
            ),
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
                rate_limit_requests_per_min, rate_limit_tokens_per_min,
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
            row.rate_limit_requests_per_min,
            row.rate_limit_tokens_per_min,
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
                rate_limit_requests_per_min, rate_limit_tokens_per_min,
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
                row.rate_limit_requests_per_min,
                row.rate_limit_tokens_per_min,
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
            // The system-prompt cache hint is a request-render concern, not
            // durable state; it is not persisted and reconstructs as unset.
            system_cache: None,
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

    async fn rollup_grouped(
        &self,
        tenant_id: Uuid,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        group_by: RollupGroup,
    ) -> Result<Vec<UsageRollupRow>> {
        // Each grouping is a distinct compile-time-checked query (the GROUP BY
        // column cannot be a bind parameter). The `$2/$3 IS NULL OR …` guards
        // make the time window optional without dynamic SQL.
        let rows = match group_by {
            RollupGroup::Key => sqlx::query!(
                r#"
                SELECT
                    virtual_key_id::text AS grp,
                    COUNT(*) AS "event_count!",
                    COALESCE(SUM(input_tokens), 0)::bigint AS "input_tokens!",
                    COALESCE(SUM(output_tokens), 0)::bigint AS "output_tokens!",
                    COALESCE(SUM(cache_read_tokens), 0)::bigint AS "cache_read_tokens!",
                    COALESCE(SUM(cache_write_tokens), 0)::bigint AS "cache_write_tokens!",
                    COALESCE(SUM(cost), 0)::numeric AS "cost!"
                FROM usage_events
                WHERE tenant_id = $1
                  AND ($2::timestamptz IS NULL OR created_at >= $2)
                  AND ($3::timestamptz IS NULL OR created_at <= $3)
                GROUP BY virtual_key_id
                ORDER BY virtual_key_id
                "#,
                tenant_id,
                from,
                to,
            )
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| UsageRollupRow {
                group: r.grp,
                event_count: r.event_count,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cache_read_tokens: r.cache_read_tokens,
                cache_write_tokens: r.cache_write_tokens,
                cost: r.cost,
            })
            .collect(),
            RollupGroup::Model => sqlx::query!(
                r#"
                SELECT
                    model AS grp,
                    COUNT(*) AS "event_count!",
                    COALESCE(SUM(input_tokens), 0)::bigint AS "input_tokens!",
                    COALESCE(SUM(output_tokens), 0)::bigint AS "output_tokens!",
                    COALESCE(SUM(cache_read_tokens), 0)::bigint AS "cache_read_tokens!",
                    COALESCE(SUM(cache_write_tokens), 0)::bigint AS "cache_write_tokens!",
                    COALESCE(SUM(cost), 0)::numeric AS "cost!"
                FROM usage_events
                WHERE tenant_id = $1
                  AND ($2::timestamptz IS NULL OR created_at >= $2)
                  AND ($3::timestamptz IS NULL OR created_at <= $3)
                GROUP BY model
                ORDER BY model
                "#,
                tenant_id,
                from,
                to,
            )
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| UsageRollupRow {
                group: Some(r.grp),
                event_count: r.event_count,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cache_read_tokens: r.cache_read_tokens,
                cache_write_tokens: r.cache_write_tokens,
                cost: r.cost,
            })
            .collect(),
            RollupGroup::Conversation => sqlx::query!(
                r#"
                SELECT
                    conversation_id::text AS grp,
                    COUNT(*) AS "event_count!",
                    COALESCE(SUM(input_tokens), 0)::bigint AS "input_tokens!",
                    COALESCE(SUM(output_tokens), 0)::bigint AS "output_tokens!",
                    COALESCE(SUM(cache_read_tokens), 0)::bigint AS "cache_read_tokens!",
                    COALESCE(SUM(cache_write_tokens), 0)::bigint AS "cache_write_tokens!",
                    COALESCE(SUM(cost), 0)::numeric AS "cost!"
                FROM usage_events
                WHERE tenant_id = $1
                  AND ($2::timestamptz IS NULL OR created_at >= $2)
                  AND ($3::timestamptz IS NULL OR created_at <= $3)
                GROUP BY conversation_id
                ORDER BY conversation_id
                "#,
                tenant_id,
                from,
                to,
            )
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| UsageRollupRow {
                group: r.grp,
                event_count: r.event_count,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cache_read_tokens: r.cache_read_tokens,
                cache_write_tokens: r.cache_write_tokens,
                cost: r.cost,
            })
            .collect(),
            // Tenant grouping is gateway-wide, not tenant-scoped; it is served
            // by `rollup_by_tenant`, so a tenant-scoped request for it is empty.
            RollupGroup::Tenant => Vec::new(),
        };
        Ok(rows)
    }

    async fn rollup_by_tenant(
        &self,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> Result<Vec<UsageRollupRow>> {
        let rows = sqlx::query!(
            r#"
            SELECT
                tenant_id::text AS "grp!",
                COUNT(*) AS "event_count!",
                COALESCE(SUM(input_tokens), 0)::bigint AS "input_tokens!",
                COALESCE(SUM(output_tokens), 0)::bigint AS "output_tokens!",
                COALESCE(SUM(cache_read_tokens), 0)::bigint AS "cache_read_tokens!",
                COALESCE(SUM(cache_write_tokens), 0)::bigint AS "cache_write_tokens!",
                COALESCE(SUM(cost), 0)::numeric AS "cost!"
            FROM usage_events
            WHERE ($1::timestamptz IS NULL OR created_at >= $1)
              AND ($2::timestamptz IS NULL OR created_at <= $2)
            GROUP BY tenant_id
            ORDER BY tenant_id
            "#,
            from,
            to,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| UsageRollupRow {
                group: Some(r.grp),
                event_count: r.event_count,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cache_read_tokens: r.cache_read_tokens,
                cache_write_tokens: r.cache_write_tokens,
                cost: r.cost,
            })
            .collect())
    }
}

#[async_trait]
impl PricingStore for PgStore {
    async fn get_effective_price(
        &self,
        provider: &str,
        model: &str,
        at: DateTime<Utc>,
    ) -> Result<Option<ModelPrice>> {
        let row = sqlx::query!(
            r#"
            SELECT
                id, provider, model, input_per_mtok, output_per_mtok,
                cache_write_per_mtok, cache_read_per_mtok, server_tool_prices,
                currency, effective_from, created_at
            FROM model_prices
            WHERE provider = $1 AND model = $2 AND effective_from <= $3
            ORDER BY effective_from DESC
            LIMIT 1
            "#,
            provider,
            model,
            at,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| ModelPrice {
            id: row.id,
            provider: row.provider,
            model: row.model,
            input_per_mtok: row.input_per_mtok,
            output_per_mtok: row.output_per_mtok,
            cache_write_per_mtok: row.cache_write_per_mtok,
            cache_read_per_mtok: row.cache_read_per_mtok,
            server_tool_prices: row.server_tool_prices,
            currency: row.currency,
            effective_from: row.effective_from,
            created_at: row.created_at,
        }))
    }

    async fn upsert_price(&self, price: NewModelPrice) -> Result<ModelPrice> {
        let id = Uuid::new_v4();
        let row = sqlx::query!(
            r#"
            INSERT INTO model_prices (
                id, provider, model, input_per_mtok, output_per_mtok,
                cache_write_per_mtok, cache_read_per_mtok, server_tool_prices,
                currency, effective_from
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT ON CONSTRAINT uq_model_prices_version
            DO UPDATE SET
                input_per_mtok = EXCLUDED.input_per_mtok,
                output_per_mtok = EXCLUDED.output_per_mtok,
                cache_write_per_mtok = EXCLUDED.cache_write_per_mtok,
                cache_read_per_mtok = EXCLUDED.cache_read_per_mtok,
                server_tool_prices = EXCLUDED.server_tool_prices,
                currency = EXCLUDED.currency
            RETURNING
                id, provider, model, input_per_mtok, output_per_mtok,
                cache_write_per_mtok, cache_read_per_mtok, server_tool_prices,
                currency, effective_from, created_at
            "#,
            id,
            price.provider,
            price.model,
            price.input_per_mtok,
            price.output_per_mtok,
            price.cache_write_per_mtok,
            price.cache_read_per_mtok,
            price.server_tool_prices,
            price.currency,
            price.effective_from,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(ModelPrice {
            id: row.id,
            provider: row.provider,
            model: row.model,
            input_per_mtok: row.input_per_mtok,
            output_per_mtok: row.output_per_mtok,
            cache_write_per_mtok: row.cache_write_per_mtok,
            cache_read_per_mtok: row.cache_read_per_mtok,
            server_tool_prices: row.server_tool_prices,
            currency: row.currency,
            effective_from: row.effective_from,
            created_at: row.created_at,
        })
    }
}

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

#[async_trait]
impl BudgetStore for PgStore {
    async fn get_tenant_budget(&self, tenant_id: Uuid) -> Result<Option<Budget>> {
        let row = sqlx::query!(
            r#"
            SELECT budget_limit_amount, budget_window, budget_action
            FROM tenants
            WHERE id = $1
            "#,
            tenant_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|row| {
            build_budget(
                row.budget_limit_amount,
                row.budget_window,
                row.budget_action,
            )
        }))
    }

    async fn set_tenant_budget(&self, tenant_id: Uuid, budget: Option<Budget>) -> Result<bool> {
        let (limit_amount, window, action) = match budget {
            Some(b) => (
                Some(b.limit_amount),
                Some(b.window.as_str().to_owned()),
                Some(b.action.as_str().to_owned()),
            ),
            None => (None, None, None),
        };
        let result = sqlx::query!(
            r#"
            UPDATE tenants
            SET budget_limit_amount = $2, budget_window = $3, budget_action = $4
            WHERE id = $1
            "#,
            tenant_id,
            limit_amount,
            window,
            action,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn set_key_budget(&self, key_id: Uuid, budget: Option<Budget>) -> Result<bool> {
        let (limit_amount, window, action) = match budget {
            Some(b) => (
                Some(b.limit_amount),
                Some(b.window.as_str().to_owned()),
                Some(b.action.as_str().to_owned()),
            ),
            None => (None, None, None),
        };
        let result = sqlx::query!(
            r#"
            UPDATE virtual_keys
            SET budget_limit_amount = $2, budget_window = $3, budget_action = $4
            WHERE id = $1
            "#,
            key_id,
            limit_amount,
            window,
            action,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn set_key_rate_limit(
        &self,
        key_id: Uuid,
        rate_limit: Option<RateLimit>,
    ) -> Result<bool> {
        let (requests, tokens) = match rate_limit {
            Some(r) => (r.requests_per_min, r.tokens_per_min),
            None => (None, None),
        };
        let result = sqlx::query!(
            r#"
            UPDATE virtual_keys
            SET rate_limit_requests_per_min = $2, rate_limit_tokens_per_min = $3
            WHERE id = $1
            "#,
            key_id,
            requests,
            tokens,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn budget_spend(
        &self,
        tenant_id: Uuid,
        key_id: Option<Uuid>,
        since: Option<DateTime<Utc>>,
    ) -> Result<Decimal> {
        let row = sqlx::query!(
            r#"
            SELECT COALESCE(SUM(cost), 0)::numeric AS "spend!"
            FROM usage_events
            WHERE tenant_id = $1
              AND ($2::uuid IS NULL OR virtual_key_id = $2)
              AND ($3::timestamptz IS NULL OR created_at >= $3)
            "#,
            tenant_id,
            key_id,
            since,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.spend)
    }
}
