//! Row and input types exchanged with the store traits.
//!
//! These are plain data types: the "New*" structs describe an insertion, and
//! the remaining structs mirror a persisted row. Conversation history is not
//! modelled here — it round-trips the [`loom_core`] domain model directly.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A persisted tenant — the unit of multi-tenant isolation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenant {
    /// The tenant's unique identifier.
    pub id: Uuid,
    /// A stable, URL-safe unique handle for the tenant.
    pub slug: String,
    /// A human-readable display name.
    pub name: String,
    /// Lifecycle status (e.g. `"active"`, `"suspended"`).
    pub status: String,
    /// When the tenant was created.
    pub created_at: DateTime<Utc>,
}

/// The fields required to create a [`Tenant`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTenant {
    /// A stable, URL-safe unique handle for the tenant.
    pub slug: String,
    /// A human-readable display name.
    pub name: String,
    /// Lifecycle status. Use `"active"` for a normal tenant.
    pub status: String,
}

impl NewTenant {
    /// Constructs a new tenant description with `status` defaulted to
    /// `"active"`.
    #[must_use]
    pub fn new(slug: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            slug: slug.into(),
            name: name.into(),
            status: "active".to_owned(),
        }
    }
}

/// A spend budget attachable at the tenant or the key level.
///
/// All three fields are stored together: a scope either has a complete budget or
/// none at all. A key-level budget overrides its tenant's default (see
/// [`BudgetStore`](crate::BudgetStore)).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    /// The spend limit, in the gateway's accounting currency.
    pub limit_amount: Decimal,
    /// The rolling window the limit applies over.
    pub window: BudgetWindow,
    /// What to do when the limit is reached.
    pub action: BudgetAction,
}

/// The rolling window a [`Budget`] limit applies over.
///
/// [`Daily`](Self::Daily), [`Weekly`](Self::Weekly) and
/// [`Monthly`](Self::Monthly) are rolling look-back windows;
/// [`Total`](Self::Total) is all-time (no lower bound).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetWindow {
    /// The trailing 24 hours.
    Daily,
    /// The trailing 7 days.
    Weekly,
    /// The trailing 30 days.
    Monthly,
    /// All recorded usage (no lower time bound).
    Total,
}

impl BudgetWindow {
    /// The stored text form (`"daily"`, `"weekly"`, `"monthly"`, `"total"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
            Self::Total => "total",
        }
    }

    /// Parses the stored text form, or `None` if it is not a known window.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            "total" => Some(Self::Total),
            _ => None,
        }
    }

    /// The inclusive lower bound of the window relative to `now`, or `None` for
    /// [`Total`](Self::Total) (an open lower bound — all recorded usage).
    #[must_use]
    pub fn start(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Daily => Some(now - chrono::Duration::days(1)),
            Self::Weekly => Some(now - chrono::Duration::weeks(1)),
            Self::Monthly => Some(now - chrono::Duration::days(30)),
            Self::Total => None,
        }
    }
}

/// What to do when a [`Budget`] limit is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetAction {
    /// Reject further spend with a `402` structured error.
    Block,
    /// Allow the request but flag it (a warning header and a logged event).
    Warn,
}

impl BudgetAction {
    /// The stored text form (`"block"`, `"warn"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Warn => "warn",
        }
    }

    /// Parses the stored text form, or `None` if it is not a known action.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "block" => Some(Self::Block),
            "warn" => Some(Self::Warn),
            _ => None,
        }
    }
}

/// Backwards-compatible alias for the pre-#10 name of [`Budget`].
pub type KeyBudget = Budget;

/// Per-key request/token rate limits, enforced by an in-process token bucket.
///
/// Either dimension may be `None` (unlimited). Single-instance for v1;
/// distributed limiting across replicas is deferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimit {
    /// Maximum requests per minute, or `None` for unlimited.
    pub requests_per_min: Option<i64>,
    /// Maximum tokens per minute, or `None` for unlimited.
    pub tokens_per_min: Option<i64>,
}

impl RateLimit {
    /// Whether this limit constrains anything (at least one dimension set).
    #[must_use]
    pub fn is_some(&self) -> bool {
        self.requests_per_min.is_some() || self.tokens_per_min.is_some()
    }
}

/// A persisted virtual API key belonging to a tenant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualKey {
    /// The key's unique identifier.
    pub id: Uuid,
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// A cryptographic hash of the secret key material (never the secret).
    pub key_hash: String,
    /// A short, non-secret prefix used to identify the key in logs and UIs.
    pub key_prefix: String,
    /// A human-readable label.
    pub name: String,
    /// Lifecycle status (e.g. `"active"`, `"revoked"`).
    pub status: String,
    /// The authorisation scopes granted to the key.
    pub scopes: Vec<String>,
    /// An optional spend budget (overrides the tenant default).
    pub budget: Option<Budget>,
    /// An optional per-key rate limit.
    pub rate_limit: Option<RateLimit>,
    /// When the key was created.
    pub created_at: DateTime<Utc>,
    /// When the key was last used to authenticate a request, if ever.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// The fields required to create a [`VirtualKey`].
#[derive(Clone, Debug, PartialEq)]
pub struct NewVirtualKey {
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// A cryptographic hash of the secret key material.
    pub key_hash: String,
    /// A short, non-secret prefix identifying the key.
    pub key_prefix: String,
    /// A human-readable label.
    pub name: String,
    /// The authorisation scopes granted to the key.
    pub scopes: Vec<String>,
    /// An optional spend budget.
    pub budget: Option<Budget>,
}

/// A persisted provider credential.
///
/// A `None` [`tenant_id`](Self::tenant_id) denotes a gateway-global credential
/// shared by all tenants that do not supply their own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCredential {
    /// The credential's unique identifier.
    pub id: Uuid,
    /// The owning tenant, or `None` for a gateway-global credential.
    pub tenant_id: Option<Uuid>,
    /// The provider this credential authenticates against (e.g. `"anthropic"`).
    pub provider: String,
    /// The encrypted secret bytes (ciphertext).
    pub encrypted_secret: Vec<u8>,
    /// The AEAD nonce used to encrypt the secret, if applicable.
    pub nonce: Option<Vec<u8>>,
    /// The additional authenticated data bound to the ciphertext, if any.
    pub aad: Option<Vec<u8>>,
    /// An optional provider base URL override.
    pub base_url: Option<String>,
    /// When the credential was created.
    pub created_at: DateTime<Utc>,
}

/// The fields required to create (or replace) a [`ProviderCredential`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewProviderCredential {
    /// The owning tenant, or `None` for a gateway-global credential.
    pub tenant_id: Option<Uuid>,
    /// The provider this credential authenticates against.
    pub provider: String,
    /// The encrypted secret bytes (ciphertext).
    pub encrypted_secret: Vec<u8>,
    /// The AEAD nonce used to encrypt the secret, if applicable.
    pub nonce: Option<Vec<u8>>,
    /// The additional authenticated data bound to the ciphertext, if any.
    pub aad: Option<Vec<u8>>,
    /// An optional provider base URL override.
    pub base_url: Option<String>,
}

/// A usage event to record for billing and attribution.
///
/// Token figures and the raw payload are taken from a loom-core [`Usage`]
/// snapshot; the surrounding fields attribute the spend to a tenant, key,
/// conversation, provider and model.
///
/// This type is serialisable so a failed write can be parked verbatim in the
/// usage outbox (see [`OutboxEntry`]) and replayed later.
///
/// [`Usage`]: loom_core::Usage
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewUsageEvent {
    /// The tenant the usage is attributed to.
    pub tenant_id: Uuid,
    /// The virtual key that authorised the request, if known.
    pub virtual_key_id: Option<Uuid>,
    /// The conversation the usage belongs to, if any.
    pub conversation_id: Option<Uuid>,
    /// The provider that served the request.
    pub provider: String,
    /// The model that served the request.
    pub model: String,
    /// The provider-reported usage snapshot.
    pub usage: loom_core::Usage,
    /// The computed monetary cost, if pricing was available.
    pub cost: Option<Decimal>,
}

/// A persisted usage event.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageEvent {
    /// The event's unique identifier.
    pub id: Uuid,
    /// The tenant the usage is attributed to.
    pub tenant_id: Uuid,
    /// The virtual key that authorised the request, if known.
    pub virtual_key_id: Option<Uuid>,
    /// The conversation the usage belongs to, if any.
    pub conversation_id: Option<Uuid>,
    /// The provider that served the request.
    pub provider: String,
    /// The model that served the request.
    pub model: String,
    /// Input (prompt) tokens billed at the full rate.
    pub input_tokens: i64,
    /// Output (completion) tokens generated.
    pub output_tokens: i64,
    /// Input tokens served from the provider's prompt cache.
    pub cache_read_tokens: i64,
    /// Input tokens written to the provider's prompt cache.
    pub cache_write_tokens: i64,
    /// Per-tool invocation counts for provider-executed tools.
    pub server_tool_counts: serde_json::Value,
    /// The computed monetary cost, if pricing was available.
    pub cost: Option<Decimal>,
    /// The provider's raw usage payload, preserved verbatim.
    pub raw_usage: Option<serde_json::Value>,
    /// When the event was recorded.
    pub created_at: DateTime<Utc>,
}

/// An aggregated summary of usage over a set of events.
///
/// This is the minimal rollup shape needed by the persistence layer; richer
/// spend reporting is layered on top in later work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageRollup {
    /// The number of events summarised.
    pub event_count: i64,
    /// Total input tokens.
    pub input_tokens: i64,
    /// Total output tokens.
    pub output_tokens: i64,
    /// Total cache-read tokens.
    pub cache_read_tokens: i64,
    /// Total cache-write tokens.
    pub cache_write_tokens: i64,
}

/// How a usage rollup is grouped.
///
/// The tenant-scoped query API groups by [`Key`](Self::Key),
/// [`Model`](Self::Model) or [`Conversation`](Self::Conversation); the
/// gateway-wide admin query groups by [`Tenant`](Self::Tenant).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollupGroup {
    /// Group by the virtual key that authorised the usage.
    Key,
    /// Group by the model that served the usage.
    Model,
    /// Group by the conversation the usage belongs to.
    Conversation,
    /// Group by tenant (gateway-wide reporting only).
    Tenant,
}

/// One grouped row of a usage rollup: aggregate token and cost totals for a
/// single group key.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageRollupRow {
    /// The group key rendered as text — a UUID for key/conversation/tenant
    /// groupings, a model identifier for model groupings, or `None` where the
    /// grouped column was itself null (e.g. usage with no virtual key).
    pub group: Option<String>,
    /// The number of events in this group.
    pub event_count: i64,
    /// Total input tokens.
    pub input_tokens: i64,
    /// Total output tokens.
    pub output_tokens: i64,
    /// Total cache-read tokens.
    pub cache_read_tokens: i64,
    /// Total cache-write tokens.
    pub cache_write_tokens: i64,
    /// Total computed cost across the group's events (events with no computed
    /// cost contribute zero).
    pub cost: Decimal,
}

/// A versioned per-model price row.
///
/// Prices are **append-only and versioned**: a price change is a new row with a
/// later [`effective_from`](Self::effective_from), never an in-place edit. The
/// effective price for an event is the latest row whose `effective_from` is at
/// or before the event's timestamp. This preserves history so a cost computed
/// under a wrong price can be recomputed from the raw usage later.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPrice {
    /// The row's unique identifier.
    pub id: Uuid,
    /// The provider the price applies to (e.g. `"anthropic"`).
    pub provider: String,
    /// The model the price applies to.
    pub model: String,
    /// USD price per million input (prompt) tokens.
    pub input_per_mtok: Decimal,
    /// USD price per million output (completion) tokens.
    pub output_per_mtok: Decimal,
    /// USD price per million tokens written to the prompt cache.
    pub cache_write_per_mtok: Decimal,
    /// USD price per million tokens read from the prompt cache.
    pub cache_read_per_mtok: Decimal,
    /// Per-request prices for provider-executed server tools, keyed by the
    /// usage field name (e.g. `{"web_search_requests": 0.01}`).
    pub server_tool_prices: serde_json::Value,
    /// ISO 4217 currency code (e.g. `"USD"`).
    pub currency: String,
    /// The instant from which this price is in effect.
    pub effective_from: DateTime<Utc>,
    /// When the row was created.
    pub created_at: DateTime<Utc>,
}

/// The fields required to insert a [`ModelPrice`] version.
#[derive(Clone, Debug, PartialEq)]
pub struct NewModelPrice {
    /// The provider the price applies to.
    pub provider: String,
    /// The model the price applies to.
    pub model: String,
    /// USD price per million input tokens.
    pub input_per_mtok: Decimal,
    /// USD price per million output tokens.
    pub output_per_mtok: Decimal,
    /// USD price per million cache-write tokens.
    pub cache_write_per_mtok: Decimal,
    /// USD price per million cache-read tokens.
    pub cache_read_per_mtok: Decimal,
    /// Per-request server-tool prices as JSON.
    pub server_tool_prices: serde_json::Value,
    /// ISO 4217 currency code.
    pub currency: String,
    /// The instant from which this price is in effect.
    pub effective_from: DateTime<Utc>,
}

/// A usage event parked in the outbox because its primary write did not
/// complete.
///
/// The full [`NewUsageEvent`] is preserved verbatim in
/// [`payload`](Self::payload) so a drain pass can replay it unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct OutboxEntry {
    /// The outbox row's unique identifier.
    pub id: Uuid,
    /// The parked usage event, exactly as it would have been recorded.
    pub payload: NewUsageEvent,
    /// Lifecycle status: `"pending"` or `"processed"`.
    pub status: String,
    /// How many drain attempts have been made.
    pub attempts: i32,
    /// The last error observed while draining, if any.
    pub last_error: Option<String>,
    /// When the entry was parked.
    pub created_at: DateTime<Utc>,
}
