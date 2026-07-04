//! [`TenantStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::model::{NewTenant, Tenant};
use crate::store::TenantStore;

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
