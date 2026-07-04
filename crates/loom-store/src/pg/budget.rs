//! [`BudgetStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::{build_budget, PgStore};
use crate::error::Result;
use crate::model::{Budget, RateLimit};
use crate::store::BudgetStore;

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
