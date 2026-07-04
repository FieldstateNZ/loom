//! Shared JSON/text HTTP plumbing reused by the Messages path and the Message
//! Batches surface (see `crate::batch`), so both honour the same auth, retry
//! and error-mapping behaviour.

use loom_provider::ProviderError;
use serde_json::Value;
use tokio::time::sleep;

use super::send::{api_error, is_retryable_status, retry_after};
use super::AnthropicProvider;

impl AnthropicProvider {
    /// Sends a JSON request (`GET`/`POST`) to an absolute `url` with the standard
    /// auth headers, retrying retryable failures with exponential backoff, and
    /// returns the parsed response body. A `POST` carries `body` as JSON; a `GET`
    /// passes `None`.
    ///
    /// Shared by the Messages path and the Message Batches surface so both honour
    /// the same auth, retry and error-mapping behaviour.
    pub(crate) async fn send_json_request(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<&Value>,
    ) -> Result<Value, ProviderError> {
        let mut attempt: u32 = 0;
        loop {
            let mut request = self
                .http
                .request(method.clone(), url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", &self.version);
            if let Some(body) = body {
                request = request
                    .header("content-type", "application/json")
                    .json(body);
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return response
                            .json::<Value>()
                            .await
                            .map_err(|error| ProviderError::Serialization(error.to_string()));
                    }
                    let code = status.as_u16();
                    let retry_after = retry_after(response.headers());
                    let payload = response.text().await.unwrap_or_default();
                    if attempt < self.max_retries && is_retryable_status(code) {
                        attempt += 1;
                        sleep(self.backoff(attempt, retry_after)).await;
                        continue;
                    }
                    return Err(api_error(code, &payload));
                }
                Err(error) => {
                    if attempt < self.max_retries && (error.is_timeout() || error.is_connect()) {
                        attempt += 1;
                        sleep(self.backoff(attempt, None)).await;
                        continue;
                    }
                    return Err(ProviderError::Transport(error.to_string()));
                }
            }
        }
    }

    /// `GET`s an absolute `url` with the standard auth headers and returns the
    /// raw response body as text, retrying retryable failures. Used to fetch a
    /// batch's JSONL results document.
    pub(crate) async fn get_text(&self, url: &str) -> Result<String, ProviderError> {
        let mut attempt: u32 = 0;
        loop {
            let request = self
                .http
                .get(url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", &self.version);
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return response
                            .text()
                            .await
                            .map_err(|error| ProviderError::Transport(error.to_string()));
                    }
                    let code = status.as_u16();
                    let retry_after = retry_after(response.headers());
                    let payload = response.text().await.unwrap_or_default();
                    if attempt < self.max_retries && is_retryable_status(code) {
                        attempt += 1;
                        sleep(self.backoff(attempt, retry_after)).await;
                        continue;
                    }
                    return Err(api_error(code, &payload));
                }
                Err(error) => {
                    if attempt < self.max_retries && (error.is_timeout() || error.is_connect()) {
                        attempt += 1;
                        sleep(self.backoff(attempt, None)).await;
                        continue;
                    }
                    return Err(ProviderError::Transport(error.to_string()));
                }
            }
        }
    }
}
