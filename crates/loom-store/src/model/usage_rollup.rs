//! Aggregated usage-rollup summaries and grouping.

use rust_decimal::Decimal;

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
    /// The portion of [`cost`](Self::cost) from batch-tier (asynchronous) usage
    /// — events with `is_batch = true`. Together with
    /// [`interactive_cost`](Self::interactive_cost) this splits the group's
    /// spend so batch and interactive usage can be told apart in a rollup.
    pub batch_cost: Decimal,
    /// The portion of [`cost`](Self::cost) from interactive usage — events with
    /// `is_batch = false`.
    pub interactive_cost: Decimal,
}
