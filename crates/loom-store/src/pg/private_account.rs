//! Atomic initial tenant/key provisioning with immutable replay receipts.
use super::PgStore;
use crate::{NewPrivateAccount, PrivateAccount, PrivateAccountStore, Result};
use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

#[async_trait]
impl PrivateAccountStore for PgStore {
    async fn provision_private_account(
        &self,
        new: NewPrivateAccount,
    ) -> Result<Option<PrivateAccount>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        // All replicas serialize this namespace, including the first insert.
        // Hash collisions only serialize unrelated accounts; they cannot merge them.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 827413))")
            .bind(&new.namespace)
            .execute(&mut *tx)
            .await?;
        let prior=sqlx::query("SELECT p.operation_id,p.request_hash,p.tenant_id,p.key_id,p.created_at,t.status AS tenant_status,t.slug,k.status AS key_status,k.tenant_id AS key_tenant,k.key_hash,k.key_prefix FROM private_accounts p JOIN tenants t ON t.id=p.tenant_id JOIN virtual_keys k ON k.id=p.key_id WHERE p.namespace=$1 FOR SHARE OF p,t,k")
            .bind(&new.namespace).fetch_optional(&mut *tx).await?;
        if let Some(row) = prior {
            if row.get::<Uuid, _>("operation_id") != new.operation_id
                || row.get::<String, _>("request_hash") != new.request_hash
                || row.get::<String, _>("tenant_status") != "active"
                || row.get::<String, _>("key_status") != "active"
                || row.get::<String, _>("slug") != new.namespace
                || row.get::<String, _>("key_hash") != new.key_hash
                || row.get::<String, _>("key_prefix") != new.key_prefix
                || row.get::<Uuid, _>("key_tenant") != row.get::<Uuid, _>("tenant_id")
            {
                tx.rollback().await?;
                return Ok(None);
            }
            let account = PrivateAccount {
                namespace: new.namespace,
                operation_id: new.operation_id,
                tenant_id: row.get("tenant_id"),
                key_id: row.get("key_id"),
                created_at: row.get("created_at"),
                created: false,
            };
            tx.commit().await?;
            return Ok(Some(account));
        }
        let tenant_id = Uuid::new_v4();
        let key_id = Uuid::new_v4();
        // A pre-existing slug/hash/operation is never adopted. ON CONFLICT on
        // every insert also covers independent legacy administrative writers.
        let tenant=sqlx::query("INSERT INTO tenants(id,slug,name) VALUES($1,$2,'Tandem private account') ON CONFLICT DO NOTHING")
            .bind(tenant_id).bind(&new.namespace).execute(&mut *tx).await?;
        if tenant.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(None);
        }
        let key=sqlx::query("INSERT INTO virtual_keys(id,tenant_id,key_hash,key_prefix,name) VALUES($1,$2,$3,$4,'Tandem application key') ON CONFLICT DO NOTHING")
            .bind(key_id).bind(tenant_id).bind(&new.key_hash).bind(&new.key_prefix).execute(&mut *tx).await?;
        if key.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(None);
        }
        let receipt=sqlx::query("INSERT INTO private_accounts(namespace,operation_id,request_hash,tenant_id,key_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING RETURNING created_at")
            .bind(&new.namespace).bind(new.operation_id).bind(&new.request_hash).bind(tenant_id).bind(key_id).fetch_optional(&mut *tx).await?;
        let Some(receipt) = receipt else {
            tx.rollback().await?;
            return Ok(None);
        };
        let account = PrivateAccount {
            namespace: new.namespace,
            operation_id: new.operation_id,
            tenant_id,
            key_id,
            created_at: receipt.get("created_at"),
            created: true,
        };
        tx.commit().await?;
        Ok(Some(account))
    }
}
