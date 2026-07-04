//! [`AnthropicProvider`]'s Message Batches methods: create, poll, cancel, and
//! fetch results.

use serde_json::{json, Value};

use loom_provider::ProviderError;

use crate::provider::AnthropicProvider;

use super::request::BatchRequest;
use super::result::AnthropicBatchResult;
use super::snapshot::AnthropicBatch;

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
