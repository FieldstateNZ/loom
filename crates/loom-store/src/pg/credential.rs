//! [`CredentialStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::model::{NewProviderCredential, ProviderCredential};
use crate::store::CredentialStore;

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
