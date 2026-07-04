//! A batch item's per-item terminal status and its persisted row.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The per-item terminal outcome within a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchItemStatus {
    /// Not yet resolved (the batch is still processing).
    Pending,
    /// The item completed successfully; its `result` holds the message.
    Succeeded,
    /// The item failed; its `result` holds the provider error.
    Errored,
    /// The item was canceled before completion.
    Canceled,
    /// The item expired before the provider completed it.
    Expired,
}

impl BatchItemStatus {
    /// The stored text form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Errored => "errored",
            Self::Canceled => "canceled",
            Self::Expired => "expired",
        }
    }

    /// Parses the stored text form, defaulting unknown values to
    /// [`Pending`](Self::Pending).
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "succeeded" => Self::Succeeded,
            "errored" => Self::Errored,
            "canceled" => Self::Canceled,
            "expired" => Self::Expired,
            _ => Self::Pending,
        }
    }
}

/// The fields required to create one item of a
/// [`BatchJob`](crate::BatchJob).
#[derive(Clone, Debug, PartialEq)]
pub struct NewBatchItem {
    /// The caller-facing per-item correlation id (unique within the job).
    pub custom_id: String,
    /// The model the item runs against.
    pub model: String,
    /// The verbatim per-item request (the inline stateless-turn shape).
    pub request: serde_json::Value,
}

/// A persisted batch item: one request within a
/// [`BatchJob`](crate::BatchJob), plus its result once resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchItem {
    /// The item's unique identifier.
    pub id: Uuid,
    /// The owning batch job.
    pub batch_id: Uuid,
    /// The owning tenant (denormalised for tenant-scoped reads).
    pub tenant_id: Uuid,
    /// The caller-facing per-item correlation id.
    pub custom_id: String,
    /// The item's position within the job.
    pub seq: i32,
    /// The model the item runs against.
    pub model: String,
    /// The item's lifecycle status.
    pub status: BatchItemStatus,
    /// The verbatim per-item request.
    pub request: serde_json::Value,
    /// The per-item result once resolved (the assistant message on success, or
    /// the provider error on failure), or `None` while still pending.
    pub result: Option<serde_json::Value>,
    /// When the item was created.
    pub created_at: DateTime<Utc>,
}
