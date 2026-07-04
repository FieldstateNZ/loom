//! [`UsageStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::model::{NewUsageEvent, RollupGroup, UsageEvent, UsageRollup, UsageRollupRow};
use crate::store::UsageStore;

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
                server_tool_counts, cost, is_batch, raw_usage
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
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
            event.is_batch,
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
                server_tool_counts, cost, is_batch, raw_usage, created_at
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
                is_batch: row.is_batch,
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
                    COALESCE(SUM(cost), 0)::numeric AS "cost!",
                    COALESCE(SUM(cost) FILTER (WHERE is_batch), 0)::numeric AS "batch_cost!",
                    COALESCE(SUM(cost) FILTER (WHERE NOT is_batch), 0)::numeric AS "interactive_cost!"
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
                batch_cost: r.batch_cost,
                interactive_cost: r.interactive_cost,
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
                    COALESCE(SUM(cost), 0)::numeric AS "cost!",
                    COALESCE(SUM(cost) FILTER (WHERE is_batch), 0)::numeric AS "batch_cost!",
                    COALESCE(SUM(cost) FILTER (WHERE NOT is_batch), 0)::numeric AS "interactive_cost!"
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
                batch_cost: r.batch_cost,
                interactive_cost: r.interactive_cost,
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
                    COALESCE(SUM(cost), 0)::numeric AS "cost!",
                    COALESCE(SUM(cost) FILTER (WHERE is_batch), 0)::numeric AS "batch_cost!",
                    COALESCE(SUM(cost) FILTER (WHERE NOT is_batch), 0)::numeric AS "interactive_cost!"
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
                batch_cost: r.batch_cost,
                interactive_cost: r.interactive_cost,
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
                COALESCE(SUM(cost), 0)::numeric AS "cost!",
                COALESCE(SUM(cost) FILTER (WHERE is_batch), 0)::numeric AS "batch_cost!",
                COALESCE(SUM(cost) FILTER (WHERE NOT is_batch), 0)::numeric AS "interactive_cost!"
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
                batch_cost: r.batch_cost,
                interactive_cost: r.interactive_cost,
            })
            .collect())
    }
}
