//! OpenAI's native Responses API behind Loom's [`Provider`] contract.
//!
//! Loom owns history: requests use `store: false`, never a provider conversation
//! or `previous_response_id`. Function-call IDs, encrypted reasoning items and
//! unknown native items survive replay. Only implemented capabilities are
//! advertised; private MCP execution belongs to the gateway, not this adapter.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::time::Duration;

use async_trait::async_trait;
use loom_core::{Conversation, ConversationOptions, Message, Usage};
use loom_provider::{
    ensure_supported, required_capabilities, Capability, Cost, ModelDescriptor, Provider,
    ProviderDescriptor, ProviderError, TurnEventStream,
};
use serde_json::Value;

mod streaming;
pub mod translate;

/// Registry name used by conversations and encrypted provider credentials.
pub const PROVIDER_NAME: &str = "openai";

/// The conservatively supported default catalogue, verified against the
/// official model documentation on 26 October 2026. Account access is separate.
#[must_use]
pub fn catalogue() -> Vec<ModelDescriptor> {
    vec![ModelDescriptor::new(
        "gpt-6-luna",
        [
            Capability::Streaming,
            Capability::ClientTools,
            Capability::Thinking,
            Capability::Vision,
        ],
    )]
}

/// An authenticated Responses client. Secrets are deliberately not `Debug`.
///
/// No automatic retry is performed: a timeout or 5xx may occur after inference
/// or a tool request has already happened. The caller owns explicit recovery.
#[derive(Clone)]
pub struct OpenAIProvider {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    models: Vec<ModelDescriptor>,
}

impl OpenAIProvider {
    /// Constructs a client with a 120-second request timeout, ten-second
    /// connection timeout, and redirects disabled to protect credentials.
    pub fn new(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ProviderError::Other(
                "OpenAI credential must not be empty".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ProviderError::Transport("could not build OpenAI HTTP client".into()))?;
        Ok(Self {
            http,
            api_key,
            base_url: "https://api.openai.com".into(),
            models: catalogue(),
        })
    }

    /// Overrides the origin/prefix for an operator-configured gateway or local
    /// test fixture. HTTPS is required except on loopback. Credentials, query
    /// strings and fragments in a base URL are rejected.
    pub fn with_base_url(mut self, base: impl Into<String>) -> Result<Self, ProviderError> {
        let base = base.into();
        let url = reqwest::Url::parse(&base)
            .map_err(|_| ProviderError::Other("invalid OpenAI base URL".into()))?;
        let local = matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        );
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !(url.scheme() == "https" || (url.scheme() == "http" && local))
        {
            return Err(ProviderError::Other(
                "OpenAI base URL requires HTTPS or loopback HTTP".into(),
            ));
        }
        self.base_url = base.trim_end_matches('/').to_owned();
        Ok(self)
    }

    /// Replaces the operator-reviewed model catalogue. This does not discover
    /// models or claim that untested capabilities are available.
    #[must_use]
    pub fn with_models(mut self, models: Vec<ModelDescriptor>) -> Self {
        self.models = models;
        self
    }

    fn request(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
        stream: bool,
    ) -> Result<Value, ProviderError> {
        let descriptor = self.descriptor();
        let model = descriptor
            .model(&conversation.binding.model)
            .ok_or_else(|| ProviderError::ModelNotFound {
                provider: PROVIDER_NAME.into(),
                model: conversation.binding.model.clone(),
            })?;
        let mut required = required_capabilities(conversation, options);
        if stream {
            required.insert(Capability::Streaming);
        }
        ensure_supported(PROVIDER_NAME, model, &required)?;
        translate::request(conversation, options, stream)
    }

    async fn send(&self, body: &Value) -> Result<reqwest::Response, ProviderError> {
        let response = self
            .http
            .post(format!("{}/v1/responses", self.base_url))
            .bearer_auth(&self.api_key)
            .json(body)
            .send()
            .await
            .map_err(|_| {
                ProviderError::Transport("OpenAI request failed; outcome may be unknown".into())
            })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let payload = response.json::<Value>().await.ok();
            return Err(ProviderError::Api {
                status: Some(status),
                message: "OpenAI rejected the request".into(),
                payload,
            });
        }
        Ok(response)
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor::new(PROVIDER_NAME, self.models.clone())
    }

    async fn complete(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
    ) -> Result<Message, ProviderError> {
        let body = self.request(conversation, options, false)?;
        let native: Value = self
            .send(&body)
            .await?
            .json()
            .await
            .map_err(|_| ProviderError::Serialization("invalid OpenAI JSON response".into()))?;
        if native.get("status").and_then(Value::as_str) != Some("completed") {
            return Err(translate::response_error(native));
        }
        translate::response(&native)
    }

    async fn stream(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
    ) -> Result<TurnEventStream, ProviderError> {
        let body = self.request(conversation, options, true)?;
        let response = self.send(&body).await?;
        if !response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"))
        {
            return Err(ProviderError::Serialization(
                "OpenAI did not return an SSE stream".into(),
            ));
        }
        Ok(streaming::event_stream(response))
    }

    fn count_cost(&self, _usage: &Usage, _model: &str) -> Cost {
        // Pricing is owned by Loom's configured price table, not this adapter.
        Cost::zero("USD")
    }
}
