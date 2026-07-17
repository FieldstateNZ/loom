//! Persistence for agent definitions and their immutable version snapshots.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::model::NewAgentDefinition;
use loom_core::{AgentContent, AgentDefinition, AgentDefinitionVersion};

/// Read/write access to [`AgentDefinition`]s, scoped to a tenant.
///
/// Definitions carry a draft and a published version pointer into immutable
/// [`AgentDefinitionVersion`] snapshots. Editing advances the draft and records
/// a new snapshot; [`publish`](AgentStore::publish) advances the published
/// pointer to the current draft, monotonically.
#[async_trait]
pub trait AgentStore {
    /// Creates a definition at draft version `1`, recording its first content
    /// snapshot. The published pointer starts unset.
    async fn create_definition(
        &self,
        tenant_id: Uuid,
        def: NewAgentDefinition,
    ) -> Result<AgentDefinition>;

    /// Loads a definition by id, scoped to a tenant. Returns `None` if it does
    /// not exist or belongs to another tenant.
    async fn get_definition(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<AgentDefinition>>;

    /// Records an edit: advances the draft version by one and stores `content`
    /// as the new draft snapshot. Returns the new draft version, or `None` if
    /// the definition does not exist or belongs to another tenant.
    ///
    /// The published pointer is untouched — an edit is never implicitly
    /// published.
    async fn update_definition(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        content: AgentContent,
    ) -> Result<Option<i32>>;

    /// Publishes the current draft: sets `published_version := draft_version`.
    /// Monotonic (the draft only ever advances) and idempotent (publishing an
    /// already-published draft is a no-op at the same version). Returns the
    /// published version, or `None` if the definition does not exist or belongs
    /// to another tenant. Never mutates any session or conversation.
    async fn publish(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<i32>>;

    /// Loads one immutable version snapshot, scoped to a tenant. Returns `None`
    /// if the definition or version does not exist or belongs to another tenant.
    async fn get_version(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        version: i32,
    ) -> Result<Option<AgentDefinitionVersion>>;
}
