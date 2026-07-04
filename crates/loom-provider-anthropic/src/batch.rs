//! Anthropic's [Message Batches API] surface for asynchronous bulk processing.
//!
//! A batch is a set of independent Messages requests, each tagged with a
//! caller-chosen `custom_id`, submitted together and processed asynchronously at
//! Anthropic's discounted batch tier. These methods reuse the provider's
//! existing [`reqwest::Client`], auth headers, base-URL override and retry
//! policy (see [`AnthropicProvider::send_json_request`]).
//!
//! The lifecycle is:
//!
//! 1. [`create_batch`](AnthropicProvider::create_batch) — submit the requests;
//! 2. [`get_batch`](AnthropicProvider::get_batch) — poll `processing_status`
//!    until it reaches `ended`;
//! 3. [`fetch_batch_results`](AnthropicProvider::fetch_batch_results) — download
//!    the JSONL results document from the batch's `results_url`;
//! 4. [`cancel_batch`](AnthropicProvider::cancel_batch) — request cancellation.
//!
//! The `params` of each request is a native Messages request body — exactly what
//! [`translate_request`](crate::translate::translate_request) produces — so
//! every Messages feature (tools, caching, thinking, …) is available in a batch
//! without a separate translation path.
//!
//! [Message Batches API]: https://docs.anthropic.com/en/api/creating-message-batches

use serde_json::{json, Value};

use loom_provider::ProviderError;

use crate::provider::AnthropicProvider;

/// One request within a batch: a caller-chosen correlation id plus the native
/// Messages request body to run.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchRequest {
    /// The caller-facing correlation id, echoed on the matching result. Unique
    /// within the batch.
    pub custom_id: String,
    /// The native Messages request body (as produced by
    /// [`translate_request`](crate::translate::translate_request)).
    pub params: Value,
}

/// Per-status request counts reported for a batch, mirroring Anthropic's
/// `request_counts` object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatchRequestCounts {
    /// Requests still being processed.
    pub processing: i64,
    /// Requests that completed successfully.
    pub succeeded: i64,
    /// Requests that errored.
    pub errored: i64,
    /// Requests that were canceled.
    pub canceled: i64,
    /// Requests that expired.
    pub expired: i64,
}

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
    fn from_native(value: &Value) -> Self {
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

/// One line of a batch's JSONL results document: a request's `custom_id` and its
/// verbatim `result` object (`{ "type": "succeeded", "message": … }`,
/// `{ "type": "errored", "error": … }`, `canceled`, or `expired`).
#[derive(Clone, Debug, PartialEq)]
pub struct AnthropicBatchResult {
    /// The correlation id of the request this result belongs to.
    pub custom_id: String,
    /// The verbatim result object.
    pub result: Value,
}

impl AnthropicProvider {
    /// The `/v1/messages/batches` base URL for this provider.
    fn batches_url(&self) -> String {
        format!(
            "{}/v1/messages/batches",
            self.base_url().trim_end_matches('/')
        )
    }

    /// Creates a batch from `requests`, returning the initial batch snapshot.
    ///
    /// # Errors
    ///
    /// Returns a [`ProviderError`] on transport failure or a non-success
    /// response (the native error envelope is preserved in
    /// [`ProviderError::Api`]).
    pub async fn create_batch(
        &self,
        requests: &[BatchRequest],
    ) -> Result<AnthropicBatch, ProviderError> {
        let body = json!({
            "requests": requests
                .iter()
                .map(|r| json!({ "custom_id": r.custom_id, "params": r.params }))
                .collect::<Vec<_>>(),
        });
        let native = self
            .send_json_request(reqwest::Method::POST, &self.batches_url(), Some(&body))
            .await?;
        Ok(AnthropicBatch::from_native(&native))
    }

    /// Polls a batch's current status by its provider-native id.
    ///
    /// # Errors
    ///
    /// Returns a [`ProviderError`] on transport failure or a non-success
    /// response.
    pub async fn get_batch(&self, id: &str) -> Result<AnthropicBatch, ProviderError> {
        let url = format!("{}/{id}", self.batches_url());
        let native = self
            .send_json_request(reqwest::Method::GET, &url, None)
            .await?;
        Ok(AnthropicBatch::from_native(&native))
    }

    /// Requests cancellation of a batch by its provider-native id, returning the
    /// updated snapshot. Cancellation is asynchronous: the batch moves to
    /// `canceling` and settles at `ended` once in-flight requests drain.
    ///
    /// # Errors
    ///
    /// Returns a [`ProviderError`] on transport failure or a non-success
    /// response.
    pub async fn cancel_batch(&self, id: &str) -> Result<AnthropicBatch, ProviderError> {
        let url = format!("{}/{id}/cancel", self.batches_url());
        let native = self
            .send_json_request(reqwest::Method::POST, &url, Some(&json!({})))
            .await?;
        Ok(AnthropicBatch::from_native(&native))
    }

    /// Fetches and parses a batch's JSONL results document from its
    /// `results_url`.
    ///
    /// Each non-empty line is one [`AnthropicBatchResult`]. Blank lines are
    /// skipped; a line that is not valid JSON fails the whole retrieval with
    /// [`ProviderError::Serialization`] rather than silently dropping a result.
    ///
    /// # Errors
    ///
    /// Returns a [`ProviderError`] on transport failure, a non-success response,
    /// or a malformed results line.
    pub async fn fetch_batch_results(
        &self,
        results_url: &str,
    ) -> Result<Vec<AnthropicBatchResult>, ProviderError> {
        let body = self.get_text(results_url).await?;
        let mut results = Vec::new();
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line)
                .map_err(|error| ProviderError::Serialization(error.to_string()))?;
            let custom_id = value
                .get("custom_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let result = value.get("result").cloned().unwrap_or(Value::Null);
            results.push(AnthropicBatchResult { custom_id, result });
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
