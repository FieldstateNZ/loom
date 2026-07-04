//! [`KeyStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::{build_budget, PgStore};
use crate::error::Result;
use crate::model::{NewVirtualKey, RateLimit, VirtualKey};
use crate::store::KeyStore;

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
