//! Typed, tenant-scoped store traits.
//!
//! These traits are the only persistence surface `loom-server` depends on, so
//! it never writes SQL. Every accessor that touches tenant-owned data takes a
//! `tenant_id` and scopes its query to it.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::model::{
    NewProviderCredential, NewTenant, NewUsageEvent, NewVirtualKey, ProviderCredential, Tenant,
    UsageEvent, UsageRollup, VirtualKey,
};
use loom_core::{Conversation, Message};

/// Persistence for tenants.
#[async_trait]
pub trait TenantStore {
    /// Creates a tenant and returns the persisted row.
    async fn create_tenant(&self, new: NewTenant) -> Result<Tenant>;

    /// Fetches a tenant by id, or `None` if it does not exist.
    async fn get_tenant(&self, id: Uuid) -> Result<Option<Tenant>>;

    /// Fetches a tenant by its unique slug, or `None` if it does not exist.
    async fn get_tenant_by_slug(&self, slug: &str) -> Result<Option<Tenant>>;
}

/// Persistence for virtual API keys.
#[async_trait]
pub trait KeyStore {
    /// Creates a virtual key and returns the persisted row.
    async fn create_key(&self, new: NewVirtualKey) -> Result<VirtualKey>;

    /// Looks a key up by its hash, or `None` if no such key exists.
    ///
    /// This is the hot authentication path.
    async fn get_key_by_hash(&self, key_hash: &str) -> Result<Option<VirtualKey>>;

    /// Marks a key revoked. Returns `true` if a key was updated.
    async fn revoke_key(&self, id: Uuid) -> Result<bool>;

    /// Records that a key was just used, updating its `last_used_at`.
    ///
    /// Returns `true` if a key was updated.
    async fn touch_key_last_used(&self, id: Uuid) -> Result<bool>;
}

/// Persistence for provider credentials.
#[async_trait]
pub trait CredentialStore {
    /// Inserts a credential, or replaces the existing one for the same
    /// `(tenant_id, provider)` pair. Returns the persisted row.
    async fn upsert_credential(&self, new: NewProviderCredential) -> Result<ProviderCredential>;

    /// Fetches the credential for a `(tenant_id, provider)` pair. Pass
    /// `tenant_id = None` to fetch the gateway-global credential.
    async fn get_credential(
        &self,
        tenant_id: Option<Uuid>,
        provider: &str,
    ) -> Result<Option<ProviderCredential>>;

    /// Lists all credentials owned by a tenant (or all global credentials when
    /// `tenant_id` is `None`).
    async fn list_credentials(&self, tenant_id: Option<Uuid>) -> Result<Vec<ProviderCredential>>;
}

/// Persistence for conversations and their message history.
///
/// History round-trips the loom-core domain model through JSONB losslessly:
/// a conversation persisted and reloaded compares equal to the original.
#[async_trait]
pub trait ConversationStore {
    /// Persists a conversation together with its current message history.
    async fn create_conversation(&self, conversation: &Conversation) -> Result<()>;

    /// Loads a conversation (with its full ordered history) by id, scoped to a
    /// tenant. Returns `None` if it does not exist or belongs to another
    /// tenant.
    async fn get_conversation(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<Conversation>>;

    /// Loads a page of a conversation's messages, ordered by sequence.
    ///
    /// `limit` caps the number of messages returned and `offset` skips that
    /// many from the start of the history.
    async fn list_messages(
        &self,
        conversation_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Message>>;

    /// Appends a message to a conversation and bumps its `updated_at`.
    ///
    /// Returns the sequence number assigned to the appended message.
    async fn append_message(&self, conversation_id: Uuid, message: &Message) -> Result<i32>;

    /// Deletes a conversation (and its messages) scoped to a tenant. Returns
    /// `true` if a conversation was deleted.
    async fn delete_conversation(&self, tenant_id: Uuid, id: Uuid) -> Result<bool>;
}

/// Persistence for usage events and their rollups.
#[async_trait]
pub trait UsageStore {
    /// Records a usage event and returns its generated id.
    async fn record_event(&self, event: NewUsageEvent) -> Result<Uuid>;

    /// Lists a tenant's usage events, most recent first, capped by `limit`.
    async fn list_events(&self, tenant_id: Uuid, limit: i64) -> Result<Vec<UsageEvent>>;

    /// Rolls a tenant's usage up into aggregate token totals.
    async fn rollup(&self, tenant_id: Uuid) -> Result<UsageRollup>;
}
