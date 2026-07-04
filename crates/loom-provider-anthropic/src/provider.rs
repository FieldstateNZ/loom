//! The [`AnthropicProvider`]: an HTTP client for Anthropic's Messages API that
//! implements the [`Provider`] trait.

use std::time::Duration;

use async_trait::async_trait;
use loom_core::{CacheNegotiation, Conversation, ConversationOptions, Message, Usage};
use loom_provider::{
    ensure_supported, required_capabilities, Capability, Cost, ModelDescriptor, Provider,
    ProviderDescriptor, ProviderError, TurnEventStream,
};
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde_json::{json, Value};
use tokio::time::sleep;

use crate::catalogue::{catalogue, PROVIDER_NAME};
use crate::{streaming, translate};

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
    /// Extra `anthropic-beta` tokens always sent, on top of the ones derived
    /// from the request's features. Lets an operator adopt a new beta (or
    /// pin an updated token) without a Loom release.
    betas: Vec<String>,
    /// Whether to auto-derive `anthropic-beta` tokens from the request's
    /// features (server tools, …). Defaults to `true`; disable to take full
    /// control of the header via [`with_beta`](Self::with_beta).
    auto_beta_headers: bool,
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
            betas: Vec::new(),
            auto_beta_headers: true,
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

    /// Adds an `anthropic-beta` token that is sent on every request, in addition
    /// to any derived from the request's features.
    ///
    /// This is the config-level override: an operator can adopt a new beta — or
    /// pin an updated token when Anthropic bumps a feature's beta — without a
    /// Loom release. Callers can also add betas per request through
    /// `provider_options["anthropic"]["betas"]`.
    #[must_use]
    pub fn with_beta(mut self, beta: impl Into<String>) -> Self {
        self.betas.push(beta.into());
        self
    }

    /// Controls whether `anthropic-beta` tokens are auto-derived from a
    /// request's features (default `true`).
    ///
    /// Disable it to suppress the catalogue-driven defaults and drive the header
    /// entirely from [`with_beta`](Self::with_beta) — the full-override path when
    /// a default token has gone stale.
    #[must_use]
    pub fn with_auto_beta_headers(mut self, enabled: bool) -> Self {
        self.auto_beta_headers = enabled;
        self
    }

    /// Computes the deterministic, de-duplicated set of `anthropic-beta` tokens
    /// to send for a request: the feature-derived tokens (when
    /// [`auto_beta_headers`](Self::auto_beta_headers) is set) plus the tokens
    /// configured on the provider.
    fn beta_headers(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
    ) -> Vec<String> {
        let mut betas: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        if self.auto_beta_headers {
            betas.extend(translate::required_betas(conversation, options));
        }
        betas.extend(self.betas.iter().cloned());
        betas.into_iter().collect()
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
    async fn send(&self, body: &Value, betas: &[String]) -> Result<Value, ProviderError> {
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
    async fn send_stream(
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

    /// Applies prompt-cache negotiation to an already-built request `body`.
    ///
    /// Cache hints are advisory: if the request carries any cache directive but
    /// the bound `model` does not declare [`Capability::PromptCaching`], the
    /// [`ConversationOptions::cache_negotiation`] policy decides the outcome —
    /// [`CacheNegotiation::SoftIgnore`] (the default) strips the `cache_control`
    /// markers and logs a warning, while [`CacheNegotiation::HardFail`] returns
    /// [`ProviderError::CapabilityUnsupported`]. When the model supports
    /// caching, the body is returned unchanged.
    fn negotiate_cache(
        &self,
        model: &ModelDescriptor,
        conversation: &Conversation,
        options: &ConversationOptions,
        mut body: Value,
    ) -> Result<Value, ProviderError> {
        if model.supports(Capability::PromptCaching)
            || !translate::requests_caching(conversation, options)
        {
            return Ok(body);
        }

        match options.cache_negotiation {
            CacheNegotiation::HardFail => Err(ProviderError::CapabilityUnsupported {
                capability: Capability::PromptCaching,
                provider: PROVIDER_NAME.to_owned(),
                model: model.id.clone(),
            }),
            _ => {
                translate::strip_cache_control(&mut body);
                tracing::warn!(
                    provider = PROVIDER_NAME,
                    model = %model.id,
                    "model does not support prompt caching; cache hints stripped (soft-ignore)"
                );
                Ok(body)
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
        let body = self.negotiate_cache(&model, conversation, options, body)?;
        let betas = self.beta_headers(conversation, options);
        let native = self.send(&body, &betas).await?;
        Ok(translate::translate_response(&native))
    }

    async fn stream(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
    ) -> Result<TurnEventStream, ProviderError> {
        let model = self.model(&conversation.binding.model)?;
        let mut required = required_capabilities(conversation, options);
        required.insert(Capability::Streaming);
        ensure_supported(PROVIDER_NAME, &model, &required)?;

        let body = translate::translate_request(conversation, options);
        let mut body = self.negotiate_cache(&model, conversation, options, body)?;
        if let Value::Object(map) = &mut body {
            map.insert("stream".into(), json!(true));
        }

        let betas = self.beta_headers(conversation, options);
        let response = self.send_stream(&body, &betas).await?;
        Ok(streaming::event_stream(response))
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

#[cfg(test)]
mod tests {
    use super::*;
    use loom_core::{CacheHint, ProviderBinding};
    use uuid::Uuid;

    /// A conversation carrying a cache hint on its system prompt, plus empty
    /// options.
    fn caching_request() -> (Conversation, ConversationOptions) {
        let mut conversation =
            Conversation::new(Uuid::new_v4(), ProviderBinding::new(PROVIDER_NAME, "m"));
        conversation.system = Some("system".to_owned());
        conversation.system_cache = Some(CacheHint::ephemeral());
        (conversation, ConversationOptions::new())
    }

    /// A model descriptor that does **not** declare prompt-caching support.
    fn model_without_caching() -> ModelDescriptor {
        ModelDescriptor::new("m", [Capability::Streaming])
    }

    #[test]
    fn negotiation_is_a_no_op_when_the_model_supports_caching() {
        let provider = AnthropicProvider::new("key").unwrap();
        let model = ModelDescriptor::new("m", [Capability::PromptCaching]);
        let (conversation, options) = caching_request();
        let body = translate::translate_request(&conversation, &options);
        let out = provider
            .negotiate_cache(&model, &conversation, &options, body.clone())
            .expect("supported models keep their cache markers");
        assert_eq!(out, body);
    }

    #[test]
    fn soft_ignore_strips_cache_markers_on_unsupported_model() {
        let provider = AnthropicProvider::new("key").unwrap();
        let model = model_without_caching();
        let (conversation, mut options) = caching_request();
        options.cache_negotiation = CacheNegotiation::SoftIgnore;
        let body = translate::translate_request(&conversation, &options);
        let out = provider
            .negotiate_cache(&model, &conversation, &options, body)
            .expect("soft-ignore continues without error");
        // The system was emitted as a cache-controlled block; after stripping it
        // carries no cache_control anywhere.
        let mut found = false;
        fn has_cache(value: &Value, found: &mut bool) {
            match value {
                Value::Object(map) => {
                    if map.contains_key("cache_control") {
                        *found = true;
                    }
                    map.values().for_each(|v| has_cache(v, found));
                }
                Value::Array(items) => items.iter().for_each(|v| has_cache(v, found)),
                _ => {}
            }
        }
        has_cache(&out, &mut found);
        assert!(!found, "soft-ignore must strip every cache_control marker");
    }

    #[test]
    fn hard_fail_rejects_cache_hints_on_unsupported_model() {
        let provider = AnthropicProvider::new("key").unwrap();
        let model = model_without_caching();
        let (conversation, mut options) = caching_request();
        options.cache_negotiation = CacheNegotiation::HardFail;
        let body = translate::translate_request(&conversation, &options);
        let err = provider
            .negotiate_cache(&model, &conversation, &options, body)
            .expect_err("hard-fail rejects the request");
        match err {
            ProviderError::CapabilityUnsupported { capability, .. } => {
                assert_eq!(capability, Capability::PromptCaching);
            }
            other => panic!("expected CapabilityUnsupported, got {other:?}"),
        }
    }

    #[test]
    fn no_caching_request_is_untouched_even_on_unsupported_model() {
        let provider = AnthropicProvider::new("key").unwrap();
        let model = model_without_caching();
        let mut conversation =
            Conversation::new(Uuid::new_v4(), ProviderBinding::new(PROVIDER_NAME, "m"));
        conversation.system = Some("system".to_owned());
        let options = ConversationOptions::new();
        let body = translate::translate_request(&conversation, &options);
        let out = provider
            .negotiate_cache(&model, &conversation, &options, body.clone())
            .expect("no cache hints, nothing to negotiate");
        assert_eq!(out, body);
    }
}
