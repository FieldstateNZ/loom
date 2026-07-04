//! A batch job's aggregate item counts, its persisted row, and its insertion
//! type.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::batch_item::NewBatchItem;
use super::batch_status::BatchStatus;

/// Aggregate per-status item counts for a batch, mirroring the provider's
/// `request_counts`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchCounts {
    /// Items still being processed.
    pub processing: i32,
    /// Items that completed successfully.
    pub succeeded: i32,
    /// Items that failed.
    pub errored: i32,
    /// Items that were canceled.
    pub canceled: i32,
    /// Items that expired.
    pub expired: i32,
}

/// A persisted batch job: a set of stateless turn requests processed
/// asynchronously at the provider's discounted batch tier.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchJob {
    /// The job's unique identifier.
    pub id: Uuid,
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// The virtual key that created the job, if known.
    pub virtual_key_id: Option<Uuid>,
    /// The provider the job runs against (e.g. `"anthropic"`).
    pub provider: String,
    /// The job's lifecycle status.
    pub status: BatchStatus,
    /// The provider-native batch identifier, once submitted.
    pub provider_batch_id: Option<String>,
    /// The provider's results URL, once the batch has ended.
    pub results_url: Option<String>,
    /// The total number of items in the job.
    pub total_items: i32,
    /// Per-status item counts.
    pub counts: BatchCounts,
    /// The last provider/poll error observed, if any (does not corrupt state).
    pub error: Option<String>,
    /// When the job was created.
    pub created_at: DateTime<Utc>,
    /// When the job was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the job reached a terminal state, if it has.
    pub ended_at: Option<DateTime<Utc>>,
}

/// The fields required to create a [`BatchJob`], together with its items.
#[derive(Clone, Debug, PartialEq)]
pub struct NewBatchJob {
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// The virtual key creating the job, if known.
    pub virtual_key_id: Option<Uuid>,
    /// The provider the job runs against.
    pub provider: String,
    /// The job's items, in submission order.
    pub items: Vec<NewBatchItem>,
}
