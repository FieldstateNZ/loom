//! Request/response DTOs for the `/v1/batches` HTTP API.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use loom_core::{CacheHint, ConversationOptions, Message};
use loom_store::{BatchCounts, BatchJob};

/// One item of a batch create request: the inline stateless-turn shape plus an
/// optional caller correlation id.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct BatchItemInput {
    /// A caller-facing correlation id, unique within the batch. Defaults to
    /// `item-{index}` when omitted.
    #[serde(default)]
    pub custom_id: Option<String>,
    /// The provider to run against (e.g. `"anthropic"`).
    pub provider: String,
    /// The model identifier, as the provider expects it.
    pub model: String,
    /// An optional system prompt.
    #[serde(default)]
    pub system: Option<String>,
    /// An optional prompt-cache breakpoint on the system prompt.
    #[serde(default)]
    #[schema(value_type = Object, nullable)]
    pub system_cache: Option<CacheHint>,
    /// The full, inline message history to run.
    #[schema(value_type = Vec<Object>)]
    pub messages: Vec<Message>,
    /// Request-time provider options.
    #[serde(default)]
    #[schema(value_type = Object, nullable)]
    pub options: Option<ConversationOptions>,
}

/// Request body for creating a batch.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBatchRequest {
    /// The batch's items, in submission order.
    pub items: Vec<BatchItemInput>,
}

/// Per-status item counts in a batch response.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchCountsDto {
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

impl From<BatchCounts> for BatchCountsDto {
    fn from(c: BatchCounts) -> Self {
        Self {
            processing: c.processing,
            succeeded: c.succeeded,
            errored: c.errored,
            canceled: c.canceled,
            expired: c.expired,
        }
    }
}

/// A batch job as returned by the API.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchJobDto {
    /// The job id.
    pub id: Uuid,
    /// The provider the job runs against.
    pub provider: String,
    /// The lifecycle status (`created`, `submitting`, `in_progress`,
    /// `canceling`, `ended`).
    pub status: String,
    /// The provider-native batch id, once submitted.
    pub provider_batch_id: Option<String>,
    /// The total number of items.
    pub total_items: i32,
    /// Per-status item counts.
    pub counts: BatchCountsDto,
    /// The last transient provider/poll error, if any.
    pub error: Option<String>,
    /// When the job was created.
    pub created_at: DateTime<Utc>,
    /// When the job was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the job ended, if it has.
    pub ended_at: Option<DateTime<Utc>>,
}

impl From<BatchJob> for BatchJobDto {
    fn from(job: BatchJob) -> Self {
        Self {
            id: job.id,
            provider: job.provider,
            status: job.status.as_str().to_owned(),
            provider_batch_id: job.provider_batch_id,
            total_items: job.total_items,
            counts: job.counts.into(),
            error: job.error,
            created_at: job.created_at,
            updated_at: job.updated_at,
            ended_at: job.ended_at,
        }
    }
}
