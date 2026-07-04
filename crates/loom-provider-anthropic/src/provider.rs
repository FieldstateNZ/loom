//! The [`AnthropicProvider`]: an HTTP client for Anthropic's Messages API that
//! implements the [`Provider`] trait.

use std::time::Duration;

use async_trait::async_trait;
use loom_core::{Conversation, ConversationOptions, Message, Usage};
use loom_provider::{
    ensure_supported, required_capabilities, Cost, ModelDescriptor, Provider, ProviderDescriptor,
    ProviderError, TurnEventStream,
};
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde_json::Value;
use tokio::time::sleep;

use crate::catalogue::{catalogue, PROVIDER_NAME};
use crate::translate;

/// The default Anthropic API base URL.
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// The default `anthropic-version` header value.
const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";
/// The default number of retries on retryable failures.
const DEFAULT_MAX_RETRIES: u32 = 3;
/// The default base delay for exponential backoff.
const DEFAULT_RETRY_BASE_DELAY: Duration = Duration::from_millis(500);
/// The default per-request timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// A [`Provider`] backed by Anthropic's Messages API.
///
/// Construct one with [`AnthropicProvider::new`] (which builds a default
/// [`reqwest::Client`]) or [`AnthropicProvider::with_http_client`] to supply
/// your own. The base URL, API version, retry budget, and backoff base delay
/// are all configurable via the `with_*` builder methods; the base URL in
/// particular can be pointed at a test server or an enterprise gateway.
///
/// Requests carry the `x-api-key`, `anthropic-version`, and
/// `content-type: application/json` headers. Responses with status `429` or
/// `5xx` are retried with exponential backoff, honouring a `retry-after`
/// header when present.
#[derive(Clone)]
pub struct AnthropicProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    version: String,
    max_retries: u32,
    retry_base_delay: Duration,
}

impl AnthropicProvider {
    /// Creates a provider authenticating with `api_key`, building a default
    /// HTTP client with a sensible timeout.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Transport`] if the underlying HTTP client cannot
    /// be constructed.
    pub fn new(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|error| ProviderError::Transport(error.to_string()))?;
        Ok(Self::with_http_client(http, api_key))
    }

    /// Creates a provider using a caller-supplied [`reqwest::Client`].
    ///
    /// Use this to control timeouts, proxies, or connection pooling.
    pub fn with_http_client(http: reqwest::Client, api_key: impl Into<String>) -> Self {
        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_owned(),
            api_key: api_key.into(),
            version: DEFAULT_ANTHROPIC_VERSION.to_owned(),
            max_retries: DEFAULT_MAX_RETRIES,
            retry_base_delay: DEFAULT_RETRY_BASE_DELAY,
        }
    }

    /// Overrides the API base URL (default `https://api.anthropic.com`).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Overrides the `anthropic-version` header value (default `2023-06-01`).
    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Overrides the number of retries attempted on retryable failures.
    #[must_use]
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Overrides the base delay used for exponential backoff.
    #[must_use]
    pub fn with_retry_base_delay(mut self, delay: Duration) -> Self {
        self.retry_base_delay = delay;
        self
    }

    /// Resolves a model from the catalogue, or fails with
    /// [`ProviderError::ModelNotFound`].
    fn model(&self, id: &str) -> Result<ModelDescriptor, ProviderError> {
        catalogue()
            .into_iter()
            .find(|model| model.id == id)
            .ok_or_else(|| ProviderError::ModelNotFound {
                provider: PROVIDER_NAME.to_owned(),
                model: id.to_owned(),
            })
    }

    /// POSTs `body` to `/v1/messages`, retrying retryable failures with
    /// exponential backoff, and returns the parsed native response.
    async fn send(&self, body: &Value) -> Result<Value, ProviderError> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let mut attempt: u32 = 0;
        loop {
            let outcome = self
                .http
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", &self.version)
                .header("content-type", "application/json")
                .json(body)
                .send()
                .await;

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

    /// Computes the backoff before retry `attempt` (1-based), honouring a
    /// server-supplied `retry-after` when present.
    fn backoff(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        retry_after.unwrap_or_else(|| {
            let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
            self.retry_base_delay.saturating_mul(factor)
        })
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor::new(PROVIDER_NAME, catalogue())
    }

    async fn complete(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
    ) -> Result<Message, ProviderError> {
        let model = self.model(&conversation.binding.model)?;
        ensure_supported(
            PROVIDER_NAME,
            &model,
            &required_capabilities(conversation, options),
        )?;

        let body = translate::translate_request(conversation, options);
        let native = self.send(&body).await?;
        Ok(translate::translate_response(&native))
    }

    async fn stream(
        &self,
        _conversation: &Conversation,
        _options: &ConversationOptions,
    ) -> Result<TurnEventStream, ProviderError> {
        // Streaming lands in issue #5; the type is kept so #5 can fill it in.
        Err(ProviderError::Other(
            "Anthropic streaming lands in issue #5".to_owned(),
        ))
    }

    fn count_cost(&self, _usage: &Usage, _model: &str) -> Cost {
        // Pricing data lands with issue #9; the hook returns a placeholder.
        Cost::zero("USD")
    }
}

/// Whether an HTTP status should be retried.
fn is_retryable_status(code: u16) -> bool {
    code == 429 || (500..=599).contains(&code)
}

/// Parses a `retry-after` header value expressed in whole seconds.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Maps an Anthropic error response into a structured [`ProviderError::Api`],
/// preserving the native error envelope verbatim in `payload`.
fn api_error(status: u16, body: &str) -> ProviderError {
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
