//! A versioned per-model price row and its insertion type.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

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
    /// The multiplier applied to token charges when the usage was served
    /// through the asynchronous batch path — the batch discount. `1.0` means no
    /// discount; Anthropic's Message Batches tier is `0.5` (50% off). Applied
    /// only to token charges, never to per-request server-tool charges.
    pub batch_multiplier: Decimal,
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
    /// The batch-tier token-charge multiplier (`1.0` = no discount, `0.5` =
    /// Anthropic's 50%-off batch tier).
    pub batch_multiplier: Decimal,
    /// ISO 4217 currency code.
    pub currency: String,
    /// The instant from which this price is in effect.
    pub effective_from: DateTime<Utc>,
}
