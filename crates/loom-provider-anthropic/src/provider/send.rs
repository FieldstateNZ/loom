//! The retryable `/v1/messages` HTTP helpers: the non-streaming POST, the
//! streaming POST, and the shared backoff/error-mapping they both use.

use std::time::Duration;

use loom_provider::ProviderError;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde_json::Value;
use tokio::time::sleep;

use super::AnthropicProvider;

impl AnthropicProvider {
    /// POSTs `body` to `/v1/messages`, retrying retryable failures with
    /// exponential backoff, and returns the parsed native response.
    pub(super) async fn send(
        &self,
        body: &Value,
        betas: &[String],
    ) -> Result<Value, ProviderError> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let mut attempt: u32 = 0;
        loop {
            let mut request = self
                .http
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", &self.version)
                .header("content-type", "application/json");
            if !betas.is_empty() {
                request = request.header("anthropic-beta", betas.join(","));
            }
            let outcome = request.json(body).send().await;

            match outcome {
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

    /// POSTs a streaming `body` to `/v1/messages`, retrying retryable failures
    /// with exponential backoff, and returns the raw [`reqwest::Response`] whose
    /// body is the SSE event stream.
    ///
    /// Retries apply only to establishing the response (status/transport); once
    /// a `2xx` stream is open, mid-stream failures are surfaced through the
    /// event stream itself rather than retried, so partial output is not lost.
    pub(super) async fn send_stream(
        &self,
        body: &Value,
        betas: &[String],
    ) -> Result<reqwest::Response, ProviderError> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let mut attempt: u32 = 0;
        loop {
            let mut request = self
                .http
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", &self.version)
                .header("content-type", "application/json")
                .header("accept", "text/event-stream");
            if !betas.is_empty() {
                request = request.header("anthropic-beta", betas.join(","));
            }
            let outcome = request.json(body).send().await;

            match outcome {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(response);
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

    /// Computes the backoff before retry `attempt` (1-based), honouring a
    /// server-supplied `retry-after` when present.
    pub(super) fn backoff(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        retry_after.unwrap_or_else(|| {
            let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
            self.retry_base_delay.saturating_mul(factor)
        })
    }
}

/// Whether an HTTP status should be retried.
pub(super) fn is_retryable_status(code: u16) -> bool {
    code == 429 || (500..=599).contains(&code)
}

/// Parses a `retry-after` header value expressed in whole seconds.
pub(super) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Maps an Anthropic error response into a structured [`ProviderError::Api`],
/// preserving the native error envelope verbatim in `payload`.
pub(super) fn api_error(status: u16, body: &str) -> ProviderError {
    match serde_json::from_str::<Value>(body) {
        Ok(payload) => {
            let message = payload
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map_or_else(
                    || format!("anthropic api error (status {status})"),
                    str::to_owned,
                );
            ProviderError::Api {
                status: Some(status),
                message,
                payload: Some(payload),
            }
        }
        Err(_) => ProviderError::Api {
            status: Some(status),
            message: if body.trim().is_empty() {
                format!("anthropic api error (status {status})")
            } else {
                body.to_owned()
            },
            payload: None,
        },
    }
}
