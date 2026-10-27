//! Opaque private credential generations and cross-process session leases.
use sqlx::{pool::PoolConnection, Postgres};
use uuid::Uuid;

use super::PgStore;
use crate::Result;

/// A connection-owned PostgreSQL advisory lease. Shared leases protect active
/// MCP clients; exclusive leases protect installation, renewal and registry
/// mutation. The connection is closed on drop, including cancellation, so a
/// session lock can never return to the pool. No transaction spans HTTP.
///
/// Each renewable logical run consumes one pool slot until its client drops.
/// Callers must bound acquisition/run time and reserve ordinary query capacity.
pub struct McpCredentialLease {
    pub(super) connection: PoolConnection<Postgres>,
    exclusive: bool,
    tenant: Uuid,
    name: String,
}

impl PgStore {
    /// Attempts a shared/exclusive lease, returning `None` immediately when
    /// another process holds a conflicting lease. Pool acquisition is governed
    /// by the pool's configured timeout (callers should also bound this future).
    pub async fn lease_mcp_credential(
        &self,
        tenant: Uuid,
        name: &str,
        exclusive: bool,
    ) -> Result<Option<McpCredentialLease>> {
        let mut connection = self.pool.acquire().await?;
        connection.close_on_drop();
        let key = format!("loom:mcp:{tenant}:{name}");
        let query = if exclusive {
            "SELECT pg_try_advisory_lock(hashtextextended($1, 0))"
        } else {
            "SELECT pg_try_advisory_lock_shared(hashtextextended($1, 0))"
        };
        let acquired: bool = sqlx::query_scalar(query)
            .bind(key)
            .fetch_one(&mut *connection)
            .await?;
        Ok(acquired.then(|| McpCredentialLease {
            connection,
            exclusive,
            tenant,
            name: name.into(),
        }))
    }

    /// Whether a tenant's registration uses the private encrypted generation
    /// store. Its existence fences all fallback to legacy static bearer auth.
    pub async fn has_private_mcp_credential(&self, tenant: Uuid, name: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM mcp_private_credentials c JOIN mcp_servers s ON s.id=c.server_id WHERE s.tenant_id=$1 AND s.name=$2)",
        ).bind(tenant).bind(name).fetch_one(&self.pool).await?)
    }
}

impl McpCredentialLease {
    /// Closes this session and waits for its advisory locks to be released.
    pub async fn release(self) -> Result<()> {
        self.connection.close().await?;
        Ok(())
    }

    /// Reads the revision and opaque ciphertext/nonce under this lease.
    pub async fn load(&mut self) -> Result<Option<(i64, Vec<u8>, Vec<u8>)>> {
        Ok(sqlx::query_as(
            "SELECT c.revision,c.ciphertext,c.nonce FROM mcp_private_credentials c JOIN mcp_servers s ON s.id=c.server_id WHERE s.tenant_id=$1 AND s.name=$2",
        ).bind(self.tenant).bind(&self.name).fetch_optional(&mut *self.connection).await?)
    }

    /// Installs the first generation (`revision=None`) or replaces exactly the
    /// observed generation. Returns the committed revision, or `None` on a
    /// missing/replaced row. Only an exclusive lease may call this method.
    pub async fn save(
        &mut self,
        revision: Option<i64>,
        ciphertext: &[u8],
        nonce: &[u8],
    ) -> Result<Option<i64>> {
        if !self.exclusive {
            return Err(sqlx::Error::Protocol("exclusive MCP lease required".into()).into());
        }
        let row = if let Some(revision) = revision {
            sqlx::query_scalar(
                "UPDATE mcp_private_credentials c SET revision=c.revision+1,ciphertext=$3,nonce=$4,updated_at=now() FROM mcp_servers s WHERE c.server_id=s.id AND s.tenant_id=$1 AND s.name=$2 AND c.revision=$5 RETURNING c.revision",
            ).bind(self.tenant).bind(&self.name).bind(ciphertext).bind(nonce).bind(revision).fetch_optional(&mut *self.connection).await?
        } else {
            sqlx::query_scalar(
                "INSERT INTO mcp_private_credentials(server_id,ciphertext,nonce) SELECT id,$3,$4 FROM mcp_servers WHERE tenant_id=$1 AND name=$2 ON CONFLICT(server_id) DO NOTHING RETURNING revision",
            ).bind(self.tenant).bind(&self.name).bind(ciphertext).bind(nonce).fetch_optional(&mut *self.connection).await?
        };
        Ok(row)
    }
}
