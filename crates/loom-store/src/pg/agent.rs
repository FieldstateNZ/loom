//! [`AgentStore`] implementation for [`PgStore`].

use async_trait::async_trait;
use uuid::Uuid;

use super::PgStore;
use crate::error::Result;
use crate::model::NewAgentDefinition;
use crate::store::AgentStore;
use loom_core::{AgentContent, AgentDefinition, AgentDefinitionVersion};

#[async_trait]
impl AgentStore for PgStore {
    async fn create_definition(
        &self,
        tenant_id: Uuid,
        def: NewAgentDefinition,
    ) -> Result<AgentDefinition> {
        let id = Uuid::new_v4();
        let content = serde_json::to_value(&def.content)?;

        let mut tx = self.pool.begin().await?;
        let row = sqlx::query!(
            r#"
            INSERT INTO agent_definitions (id, tenant_id, name, draft_version, published_version)
            VALUES ($1, $2, $3, 1, NULL)
            RETURNING id, tenant_id, name, draft_version, published_version, created_at, updated_at
            "#,
            id,
            tenant_id,
            def.name,
        )
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO agent_definition_versions (agent_definition_id, version, content)
            VALUES ($1, 1, $2)
            "#,
            id,
            content,
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(AgentDefinition {
            id: row.id,
            tenant_id: row.tenant_id,
            name: row.name,
            draft_version: row.draft_version,
            published_version: row.published_version,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    async fn get_definition(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<AgentDefinition>> {
        let row = sqlx::query!(
            r#"
            SELECT id, tenant_id, name, draft_version, published_version, created_at, updated_at
            FROM agent_definitions
            WHERE id = $1 AND tenant_id = $2
            "#,
            id,
            tenant_id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| AgentDefinition {
            id: row.id,
            tenant_id: row.tenant_id,
            name: row.name,
            draft_version: row.draft_version,
            published_version: row.published_version,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }

    async fn update_definition(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        content: AgentContent,
    ) -> Result<Option<i32>> {
        let content = serde_json::to_value(&content)?;

        let mut tx = self.pool.begin().await?;
        // Advance the draft, but only for a definition this tenant owns.
        let Some(row) = sqlx::query!(
            r#"
            UPDATE agent_definitions
            SET draft_version = draft_version + 1, updated_at = now()
            WHERE id = $1 AND tenant_id = $2
            RETURNING draft_version
            "#,
            id,
            tenant_id,
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.rollback().await?;
            return Ok(None);
        };

        sqlx::query!(
            r#"
            INSERT INTO agent_definition_versions (agent_definition_id, version, content)
            VALUES ($1, $2, $3)
            "#,
            id,
            row.draft_version,
            content,
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(row.draft_version))
    }

    async fn publish(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<i32>> {
        // published_version := draft_version. Monotonic (the draft only ever
        // advances) and idempotent. Touches no session or conversation.
        let row = sqlx::query!(
            r#"
            UPDATE agent_definitions
            SET published_version = draft_version, updated_at = now()
            WHERE id = $1 AND tenant_id = $2
            RETURNING published_version
            "#,
            id,
            tenant_id,
        )
        .fetch_optional(&self.pool)
        .await?;

        // published_version is non-null after the UPDATE.
        Ok(row.and_then(|row| row.published_version))
    }

    async fn get_version(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        version: i32,
    ) -> Result<Option<AgentDefinitionVersion>> {
        let Some(row) = sqlx::query!(
            r#"
            SELECT v.agent_definition_id, v.version, v.content, v.created_at
            FROM agent_definition_versions v
            JOIN agent_definitions d ON d.id = v.agent_definition_id
            WHERE v.agent_definition_id = $1 AND v.version = $2 AND d.tenant_id = $3
            "#,
            id,
            version,
            tenant_id,
        )
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };

        Ok(Some(AgentDefinitionVersion {
            agent_definition_id: row.agent_definition_id,
            version: row.version,
            content: serde_json::from_value(row.content)?,
            created_at: row.created_at,
        }))
    }
}
