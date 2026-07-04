//! Construction and builder-style configuration of [`AnthropicProvider`].

use std::time::Duration;

use loom_core::{Conversation, ConversationOptions};
use loom_provider::{ModelDescriptor, ProviderError};

use crate::catalogue::{catalogue, PROVIDER_NAME};
use crate::translate;

use super::{
    AnthropicProvider, DEFAULT_ANTHROPIC_VERSION, DEFAULT_MAX_RETRIES, DEFAULT_RETRY_BASE_DELAY,
    DEFAULT_TIMEOUT,
};

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
            base_url: super::DEFAULT_BASE_URL.to_owned(),
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
    pub(super) fn beta_headers(
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
    pub(super) fn model(&self, id: &str) -> Result<ModelDescriptor, ProviderError> {
        catalogue()
            .into_iter()
            .find(|model| model.id == id)
            .ok_or_else(|| ProviderError::ModelNotFound {
                provider: PROVIDER_NAME.to_owned(),
                model: id.to_owned(),
            })
    }

    /// The configured API base URL (no trailing slash guarantees).
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }
}
