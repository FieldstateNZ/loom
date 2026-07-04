//! [`AnthropicBatch`]: a snapshot of a batch's status.

use serde_json::Value;

use super::counts::BatchRequestCounts;

/// A snapshot of a batch as reported by the create/poll/cancel endpoints.
#[derive(Clone, Debug, PartialEq)]
pub struct AnthropicBatch {
    /// The provider-native batch id (`msgbatch_…`).
    pub id: String,
    /// The processing status: `in_progress`, `canceling`, or `ended`.
    pub processing_status: String,
    /// Per-status request counts.
    pub counts: BatchRequestCounts,
    /// The URL of the JSONL results document, present once the batch has ended.
    pub results_url: Option<String>,
    /// When the batch reached a terminal state (RFC 3339), if it has.
    pub ended_at: Option<String>,
}

impl AnthropicBatch {
    /// Whether the batch has reached its terminal `ended` state.
    #[must_use]
    pub fn is_ended(&self) -> bool {
        self.processing_status == "ended"
    }

    /// Parses a native batch object.
    pub(super) fn from_native(value: &Value) -> Self {
        let counts = value.get("request_counts");
        let count = |key: &str| -> i64 {
            counts
                .and_then(|c| c.get(key))
                .and_then(Value::as_i64)
                .unwrap_or(0)
        };
        Self {
            id: value
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            processing_status: value
                .get("processing_status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            counts: BatchRequestCounts {
                processing: count("processing"),
                succeeded: count("succeeded"),
                errored: count("errored"),
                canceled: count("canceled"),
                expired: count("expired"),
            },
            results_url: value
                .get("results_url")
                .and_then(Value::as_str)
                .map(str::to_owned),
            ended_at: value
                .get("ended_at")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_a_native_batch_snapshot() {
        let native = json!({
            "id": "msgbatch_123",
            "processing_status": "in_progress",
            "request_counts": {
                "processing": 2, "succeeded": 1, "errored": 0,
                "canceled": 0, "expired": 0
            },
            "results_url": null,
            "ended_at": null
        });
        let batch = AnthropicBatch::from_native(&native);
        assert_eq!(batch.id, "msgbatch_123");
        assert!(!batch.is_ended());
        assert_eq!(batch.counts.processing, 2);
        assert_eq!(batch.counts.succeeded, 1);
        assert!(batch.results_url.is_none());
    }

    #[test]
    fn ended_batch_reports_terminal() {
        let native = json!({
            "id": "msgbatch_9",
            "processing_status": "ended",
            "request_counts": { "succeeded": 3 },
            "results_url": "https://api.anthropic.com/v1/messages/batches/msgbatch_9/results",
            "ended_at": "2026-07-04T00:00:00Z"
        });
        let batch = AnthropicBatch::from_native(&native);
        assert!(batch.is_ended());
        assert_eq!(batch.counts.succeeded, 3);
        assert!(batch.results_url.is_some());
    }
}
