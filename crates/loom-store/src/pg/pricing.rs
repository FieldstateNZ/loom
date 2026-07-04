//! [`PricingStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::model::{ModelPrice, NewModelPrice};
use crate::store::PricingStore;

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
                batch_multiplier, currency, effective_from, created_at
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
            batch_multiplier: row.batch_multiplier,
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
                batch_multiplier, currency, effective_from
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT ON CONSTRAINT uq_model_prices_version
            DO UPDATE SET
                input_per_mtok = EXCLUDED.input_per_mtok,
                output_per_mtok = EXCLUDED.output_per_mtok,
                cache_write_per_mtok = EXCLUDED.cache_write_per_mtok,
                cache_read_per_mtok = EXCLUDED.cache_read_per_mtok,
                server_tool_prices = EXCLUDED.server_tool_prices,
                batch_multiplier = EXCLUDED.batch_multiplier,
                currency = EXCLUDED.currency
            RETURNING
                id, provider, model, input_per_mtok, output_per_mtok,
                cache_write_per_mtok, cache_read_per_mtok, server_tool_prices,
                batch_multiplier, currency, effective_from, created_at
            "#,
            id,
            price.provider,
            price.model,
            price.input_per_mtok,
            price.output_per_mtok,
            price.cache_write_per_mtok,
            price.cache_read_per_mtok,
            price.server_tool_prices,
            price.batch_multiplier,
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
            batch_multiplier: row.batch_multiplier,
            currency: row.currency,
            effective_from: row.effective_from,
            created_at: row.created_at,
        })
    }
}
