//! Provider-agnostic batch state: [`ProviderBatchSnapshot`] (poll status) and
//! [`ProviderBatchResult`] (per-item outcome).

use loom_core::Usage;
use loom_store::{BatchCounts, BatchItemStatus};
use serde_json::Value;

/// A provider-agnostic snapshot of a batch's state.
#[derive(Clone, Debug)]
pub struct ProviderBatchSnapshot {
    /// The provider-native batch id.
    pub provider_batch_id: String,
    /// Whether the batch has reached its terminal state.
    pub ended: bool,
    /// Per-status item counts.
    pub counts: BatchCounts,
    /// The results document location, once ended.
    pub results_url: Option<String>,
}

/// A provider-agnostic per-item result.
#[derive(Clone, Debug)]
pub struct ProviderBatchResult {
    /// The correlation id of the request this result belongs to.
    pub custom_id: String,
    /// The item's terminal outcome.
    pub outcome: BatchItemStatus,
    /// The verbatim result payload to persist (assistant message on success, or
    /// the provider error otherwise).
    pub result: Value,
    /// The parsed usage snapshot, for billing (present on success).
    pub usage: Option<Usage>,
}
