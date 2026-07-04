//! A usage event to record for billing and attribution, and its persisted row.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A usage event to record for billing and attribution.
///
/// Token figures and the raw payload are taken from a loom-core [`Usage`]
/// snapshot; the surrounding fields attribute the spend to a tenant, key,
/// conversation, provider and model.
///
/// This type is serialisable so a failed write can be parked verbatim in the
/// usage outbox (see [`OutboxEntry`](crate::OutboxEntry)) and replayed later.
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
    /// Whether this usage was served through the asynchronous batch path (and
    /// therefore priced at the discounted batch tier). Defaults to `false` so
    /// interactive turns — and outbox payloads written before this field
    /// existed — deserialise unchanged.
    #[serde(default)]
    pub is_batch: bool,
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
    /// Whether this usage was served through the asynchronous batch path.
    pub is_batch: bool,
    /// The provider's raw usage payload, preserved verbatim.
    pub raw_usage: Option<serde_json::Value>,
    /// When the event was recorded.
    pub created_at: DateTime<Utc>,
}
