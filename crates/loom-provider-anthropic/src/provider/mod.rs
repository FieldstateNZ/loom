//! The [`AnthropicProvider`]: an HTTP client for Anthropic's Messages API that
//! implements the [`Provider`] trait.
//!
//! The implementation is split across cohesive submodules: [`config`] (the
//! constructors and `with_*` builder methods), [`send`] (the retryable
//! `/v1/messages` POST helpers), [`batch_http`] (the shared JSON/text HTTP
//! plumbing reused by the Message Batches surface), [`cache_negotiation`] (the
//! prompt-cache capability check applied before dispatch), [`complete`] (the
//! non-streaming [`Provider::complete`] path) and [`stream`] (the streaming
//! [`Provider::stream`] path).
//!
//! [`Provider`]: loom_provider::Provider

use std::time::Duration;

mod batch_http;
mod cache_negotiation;
mod complete;
mod config;
mod send;
mod stream;
mod trait_impl;

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

/// A [`Provider`](loom_provider::Provider) backed by Anthropic's Messages API.
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
