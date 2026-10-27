//! [`McpServerStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::model::{McpServer, NewMcpServer};
use crate::store::McpServerStore;

#[async_trait]
impl McpServerStore for PgStore {
    async fn upsert_mcp_server(&self, new: NewMcpServer) -> Result<McpServer> {
        let mut lease = self
            .lease_mcp_credential(new.tenant_id, &new.name, true)
            .await?
            .ok_or(sqlx::Error::Protocol("MCP registration is busy".into()))?;
        if lease.load().await?.is_some() {
            return Err(sqlx::Error::Protocol(
                "private MCP registration requires controlled replacement".into(),
            )
            .into());
        }
        let id = Uuid::new_v4();
        let row = sqlx::query!(
            r#"
            INSERT INTO mcp_servers (
                id, tenant_id, name, url, encrypted_token, nonce, tool_configuration
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT ON CONSTRAINT uq_mcp_servers_tenant_name
            DO UPDATE SET
                url = EXCLUDED.url,
                encrypted_token = EXCLUDED.encrypted_token,
                nonce = EXCLUDED.nonce,
                tool_configuration = EXCLUDED.tool_configuration,
                updated_at = now()
            RETURNING
                id, tenant_id, name, url, encrypted_token, nonce,
                tool_configuration, created_at, updated_at
            "#,
            id,
            new.tenant_id,
            new.name,
            new.url,
            new.encrypted_token,
            new.nonce,
            new.tool_configuration,
        )
        .fetch_one(&mut *lease.connection)
        .await?;
        Ok(McpServer {
            id: row.id,
            tenant_id: row.tenant_id,
            name: row.name,
            url: row.url,
            encrypted_token: row.encrypted_token,
            nonce: row.nonce,
            tool_configuration: row.tool_configuration,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    async fn get_mcp_server(&self, tenant_id: Uuid, name: &str) -> Result<Option<McpServer>> {
        let row = sqlx::query!(
            r#"
            SELECT
                id, tenant_id, name, url, encrypted_token, nonce,
                tool_configuration, created_at, updated_at
            FROM mcp_servers
            WHERE tenant_id = $1 AND name = $2
            "#,
            tenant_id,
            name,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| McpServer {
            id: row.id,
            tenant_id: row.tenant_id,
            name: row.name,
            url: row.url,
            encrypted_token: row.encrypted_token,
            nonce: row.nonce,
            tool_configuration: row.tool_configuration,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }

    async fn list_mcp_servers(&self, tenant_id: Uuid) -> Result<Vec<McpServer>> {
        let rows = sqlx::query!(
            r#"
            SELECT
                id, tenant_id, name, url, encrypted_token, nonce,
                tool_configuration, created_at, updated_at
            FROM mcp_servers
            WHERE tenant_id = $1
            ORDER BY name
            "#,
            tenant_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| McpServer {
                id: row.id,
                tenant_id: row.tenant_id,
                name: row.name,
                url: row.url,
                encrypted_token: row.encrypted_token,
                nonce: row.nonce,
                tool_configuration: row.tool_configuration,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect())
    }

    async fn delete_mcp_server(&self, tenant_id: Uuid, name: &str) -> Result<bool> {
        let mut lease = self
            .lease_mcp_credential(tenant_id, name, true)
            .await?
            .ok_or(sqlx::Error::Protocol("MCP registration is busy".into()))?;
        let result = sqlx::query!(
            r#"DELETE FROM mcp_servers WHERE tenant_id = $1 AND name = $2"#,
            tenant_id,
            name,
        )
        .execute(&mut *lease.connection)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
