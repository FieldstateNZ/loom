//! Typed, tenant-scoped store traits.
//!
//! These traits are the only persistence surface `loom-server` depends on, so
//! it never writes SQL. Every accessor that touches tenant-owned data takes a
//! `tenant_id` and scopes its query to it.

use async_trait::async_trait;
use uuid::Uuid;

use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::model::{
    ModelPrice, NewModelPrice, NewProviderCredential, NewTenant, NewUsageEvent, NewVirtualKey,
    OutboxEntry, ProviderCredential, RollupGroup, Tenant, UsageEvent, UsageRollup, UsageRollupRow,
    VirtualKey,
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

    /// Loads a page of a conversation's messages, ordered by sequence, scoped
    /// to a tenant.
    ///
    /// `limit` caps the number of messages returned and `offset` skips that
    /// many from the start of the history. Returns an empty vector if the
    /// conversation does not exist or belongs to another tenant.
    async fn list_messages(
        &self,
        tenant_id: Uuid,
        conversation_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Message>>;

    /// Appends a message to a conversation and bumps its `updated_at`, scoped
    /// to a tenant.
    ///
    /// Returns `Some(seq)` with the sequence number assigned to the appended
    /// message, or `None` (a no-op) if the conversation does not exist or
    /// belongs to another tenant.
    async fn append_message(
        &self,
        tenant_id: Uuid,
        conversation_id: Uuid,
        message: &Message,
    ) -> Result<Option<i32>>;

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

    /// Rolls a tenant's usage up into grouped token and cost totals over an
    /// optional `[from, to]` time window (inclusive; `None` bounds are open).
    ///
    /// `group_by` selects the grouping dimension; passing
    /// [`RollupGroup::Tenant`] here is a caller error and yields an empty
    /// result — gateway-wide reporting uses [`Self::rollup_by_tenant`].
    async fn rollup_grouped(
        &self,
        tenant_id: Uuid,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        group_by: RollupGroup,
    ) -> Result<Vec<UsageRollupRow>>;

    /// Rolls **all** tenants' usage up, grouped by tenant, over an optional
    /// time window. Gateway-wide; not tenant-scoped — for the root-token admin
    /// query only.
    async fn rollup_by_tenant(
        &self,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> Result<Vec<UsageRollupRow>>;
}

/// Persistence for the versioned pricing model.
#[async_trait]
pub trait PricingStore {
    /// Returns the effective price for `(provider, model)` at instant `at`:
    /// the latest row whose `effective_from` is at or before `at`, or `None`
    /// if no such price is configured.
    async fn get_effective_price(
        &self,
        provider: &str,
        model: &str,
        at: DateTime<Utc>,
    ) -> Result<Option<ModelPrice>>;

    /// Inserts a price version, returning the persisted row.
    ///
    /// Prices are versioned, not overwritten: a genuine price change is a new
    /// row with a later `effective_from`. Re-inserting the exact same
    /// `(provider, model, effective_from)` corrects that one version in place
    /// (idempotent seeding).
    async fn upsert_price(&self, price: NewModelPrice) -> Result<ModelPrice>;
}

/// Persistence for the usage outbox — the failure-mode safety net.
///
/// A usage-write failure must never fail the user's turn. When the primary
/// [`UsageStore::record_event`] write fails, the event is parked here and a
/// drain pass ([`crate::drain_usage_outbox`]) replays it later.
#[async_trait]
pub trait OutboxStore {
    /// Parks a usage event in the outbox (status `pending`), returning its id.
    async fn enqueue_outbox(&self, event: &NewUsageEvent) -> Result<Uuid>;

    /// Lists pending outbox entries oldest-first, capped by `limit`.
    async fn list_pending_outbox(&self, limit: i64) -> Result<Vec<OutboxEntry>>;

    /// Marks an outbox entry processed (drained successfully).
    async fn mark_outbox_processed(&self, id: Uuid) -> Result<()>;

    /// Records a failed drain attempt: bumps the attempt count and stores the
    /// error, leaving the entry pending for a later retry.
    async fn mark_outbox_failed(&self, id: Uuid, error: &str) -> Result<()>;
}
