//! Typed, tenant-scoped store traits.
//!
//! These traits are the only persistence surface `loom-server` depends on, so
//! it never writes SQL. Every accessor that touches tenant-owned data takes a
//! `tenant_id` and scopes its query to it.

use async_trait::async_trait;
use uuid::Uuid;

use chrono::{DateTime, Utc};

use rust_decimal::Decimal;

use crate::error::Result;
use crate::model::{
    BatchCounts, BatchItem, BatchItemStatus, BatchJob, BatchStatus, Budget, McpServer, ModelPrice,
    NewBatchJob, NewMcpServer, NewModelPrice, NewProviderCredential, NewTenant, NewUsageEvent,
    NewVirtualKey, OutboxEntry, ProviderCredential, RateLimit, RollupGroup, Tenant, UsageEvent,
    UsageRollup, UsageRollupRow, VirtualKey,
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

/// Persistence for tenant-scoped MCP server registrations.
///
/// Conversations reference a server by name; the resolver loads the row,
/// decrypts its authorization token, and injects it into the provider request
/// server-side. Every accessor is scoped to a `tenant_id` so one tenant can
/// never read or delete another tenant's registration.
#[async_trait]
pub trait McpServerStore {
    /// Inserts a registration, or replaces the existing one for the same
    /// `(tenant_id, name)` pair. Returns the persisted row.
    async fn upsert_mcp_server(&self, new: NewMcpServer) -> Result<McpServer>;

    /// Fetches a tenant's registration by name, or `None` if absent.
    async fn get_mcp_server(&self, tenant_id: Uuid, name: &str) -> Result<Option<McpServer>>;

    /// Lists a tenant's registrations, ordered by name.
    async fn list_mcp_servers(&self, tenant_id: Uuid) -> Result<Vec<McpServer>>;

    /// Deletes a tenant's registration by name. Returns `true` if one was
    /// deleted.
    async fn delete_mcp_server(&self, tenant_id: Uuid, name: &str) -> Result<bool>;
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

/// Persistence for budgets and rate limits, and the current-window spend query
/// that budget enforcement reads.
///
/// Budgets attach at the tenant and the key level; a key-level budget overrides
/// the tenant default. Rate limits attach per key. Current spend is derived from
/// the `usage_events` rollup (the #9 store) at enforcement time — never
/// denormalised here.
#[async_trait]
pub trait BudgetStore {
    /// Fetches a tenant's default budget, or `None` if it has none.
    async fn get_tenant_budget(&self, tenant_id: Uuid) -> Result<Option<Budget>>;

    /// Sets (or, with `None`, clears) a tenant's default budget. Returns `true`
    /// if the tenant exists and was updated.
    async fn set_tenant_budget(&self, tenant_id: Uuid, budget: Option<Budget>) -> Result<bool>;

    /// Sets (or, with `None`, clears) a key's budget override. Returns `true`
    /// if the key exists and was updated.
    async fn set_key_budget(&self, key_id: Uuid, budget: Option<Budget>) -> Result<bool>;

    /// Sets (or, with `None`, clears) a key's rate limit. Returns `true` if the
    /// key exists and was updated.
    async fn set_key_rate_limit(&self, key_id: Uuid, rate_limit: Option<RateLimit>)
        -> Result<bool>;

    /// Sums the recorded cost of usage in the current budget window.
    ///
    /// Scoped to `tenant_id`; if `key_id` is `Some`, further scoped to that
    /// key. `since` is the inclusive lower bound on event time, or `None` for
    /// an open window (all recorded usage). Events with no computed cost
    /// contribute zero.
    async fn budget_spend(
        &self,
        tenant_id: Uuid,
        key_id: Option<Uuid>,
        since: Option<DateTime<Utc>>,
    ) -> Result<Decimal>;
}

/// Persistence for asynchronous batch jobs and their per-item results.
///
/// A batch job is a set of stateless turn requests processed asynchronously at
/// the provider's discounted batch tier. Tenant-facing accessors
/// ([`create_batch_job`](Self::create_batch_job),
/// [`get_batch_job`](Self::get_batch_job),
/// [`list_batch_items`](Self::list_batch_items),
/// [`request_batch_cancel`](Self::request_batch_cancel)) are scoped to a
/// `tenant_id`. The poll-worker accessors
/// ([`list_active_batch_jobs`](Self::list_active_batch_jobs),
/// [`get_batch_items`](Self::get_batch_items) and the update methods) are
/// gateway-wide — the worker advances every tenant's jobs — and are keyed by the
/// job's own id, which is an unguessable UUID.
///
/// Per-item results are **stored**, not fetched-through: when a batch ends the
/// worker retrieves the provider's results once and persists each into
/// [`BatchItem::result`], so reads never depend on the provider's results-URL
/// retention.
#[async_trait]
pub trait BatchStore {
    /// Creates a batch job together with its items in one transaction, and
    /// returns the persisted job (status `created`).
    async fn create_batch_job(&self, new: NewBatchJob) -> Result<BatchJob>;

    /// Fetches a job by id, scoped to a tenant. Returns `None` if it does not
    /// exist or belongs to another tenant.
    async fn get_batch_job(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<BatchJob>>;

    /// Lists a job's items in submission order, scoped to a tenant. Returns an
    /// empty vector if the job does not exist or belongs to another tenant.
    async fn list_batch_items(&self, tenant_id: Uuid, batch_id: Uuid) -> Result<Vec<BatchItem>>;

    /// Lists jobs that are still advancing (status other than `ended`),
    /// oldest-first, capped by `limit`. Gateway-wide — for the poll worker only.
    async fn list_active_batch_jobs(&self, limit: i64) -> Result<Vec<BatchJob>>;

    /// Lists a job's items in submission order by job id (not tenant-scoped) —
    /// for the poll worker building the provider submission.
    async fn get_batch_items(&self, batch_id: Uuid) -> Result<Vec<BatchItem>>;

    /// Records that a job was submitted to the provider: stores the
    /// provider-native batch id and initial counts and moves the job to
    /// `in_progress`. Clears any prior transient error.
    async fn mark_batch_submitted(
        &self,
        id: Uuid,
        provider_batch_id: &str,
        counts: BatchCounts,
    ) -> Result<()>;

    /// Applies a poll result: updates counts and status, and — when the job has
    /// ended — records the `results_url` and `ended_at`. Clears any prior
    /// transient error.
    async fn update_batch_progress(
        &self,
        id: Uuid,
        status: BatchStatus,
        counts: BatchCounts,
        results_url: Option<&str>,
        ended_at: Option<DateTime<Utc>>,
    ) -> Result<()>;

    /// Persists one item's resolved result (status + payload), keyed by
    /// `(batch_id, custom_id)`.
    async fn save_batch_item_result(
        &self,
        batch_id: Uuid,
        custom_id: &str,
        status: BatchItemStatus,
        result: &serde_json::Value,
    ) -> Result<()>;

    /// Requests cancellation of a tenant's job.
    ///
    /// A job that has not yet been submitted (`created`, no provider batch id)
    /// is finalised immediately as `ended` with every item canceled. A job the
    /// provider is already running moves to `canceling` so the worker can relay
    /// the cancellation. Returns the updated job, or `None` if it does not exist
    /// or belongs to another tenant. An already-`ended` job is returned
    /// unchanged (idempotent).
    async fn request_batch_cancel(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<BatchJob>>;

    /// Records a transient provider/poll error against a job **without** changing
    /// its status, so a provider fault never corrupts the lifecycle.
    async fn set_batch_error(&self, id: Uuid, error: &str) -> Result<()>;
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
