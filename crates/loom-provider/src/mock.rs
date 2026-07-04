//! An in-memory [`MockProvider`] for tests.
//!
//! Available under the `mock` cargo feature (and always within this crate's own
//! tests). It returns canned responses and a canned event stream, and declares
//! a configurable capability set — enough for downstream crates (such as the
//! server) to exercise provider-driven code paths without a live backend.

use async_trait::async_trait;
use futures::stream;
use loom_core::{Conversation, ConversationOptions, Message, Usage};

use crate::capability::{
    ensure_supported, required_capabilities, Capability, ModelDescriptor, ProviderDescriptor,
};
use crate::error::{Cost, ProviderError};
use crate::event::{StopReason, TurnEvent, TurnEventKind};
use crate::provider::{Provider, TurnEventStream};

/// A configurable, canned [`Provider`] implementation for use in tests.
///
/// Build one with [`MockProvider::new`], optionally overriding the canned
/// completion message and event stream. Capability negotiation runs exactly as
/// it would for a real provider, so tests can assert both the happy path and
/// [`ProviderError::CapabilityUnsupported`].
#[derive(Clone)]
pub struct MockProvider {
    name: String,
    model_id: String,
    capabilities: Vec<Capability>,
    completion: Message,
    events: Vec<TurnEvent>,
}

impl MockProvider {
    /// Creates a mock provider named `name` offering a single model `model_id`
    /// with the given `capabilities`.
    ///
    /// The default canned completion is an assistant message reading
    /// `"mock response"`, and the default canned stream is a
    /// `TurnStarted` … `TurnEnded` pair.
    pub fn new(
        name: impl Into<String>,
        model_id: impl Into<String>,
        capabilities: impl IntoIterator<Item = Capability>,
    ) -> Self {
        let name = name.into();
        let model_id = model_id.into();
        let events = vec![
            TurnEvent::new(
                TurnEventKind::TurnStarted,
                serde_json::json!({ "type": "message_start" }),
            ),
            TurnEvent::new(
                TurnEventKind::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
                serde_json::json!({ "type": "message_stop" }),
            ),
        ];
        Self {
            name,
            model_id,
            capabilities: capabilities.into_iter().collect(),
            completion: Message::assistant("mock response"),
            events,
        }
    }

    /// Overrides the canned completion message returned by
    /// [`Provider::complete`].
    #[must_use]
    pub fn with_completion(mut self, message: Message) -> Self {
        self.completion = message;
        self
    }

    /// Overrides the canned event stream returned by [`Provider::stream`].
    #[must_use]
    pub fn with_events(mut self, events: Vec<TurnEvent>) -> Self {
        self.events = events;
        self
    }

    fn model(&self) -> ModelDescriptor {
        ModelDescriptor::new(self.model_id.clone(), self.capabilities.iter().copied())
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor::new(self.name.clone(), [self.model()])
    }

    async fn complete(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
    ) -> Result<Message, ProviderError> {
        ensure_supported(
            &self.name,
            &self.model(),
            &required_capabilities(conversation, options),
        )?;
        Ok(self.completion.clone())
    }

    async fn stream(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
    ) -> Result<TurnEventStream, ProviderError> {
        let mut required = required_capabilities(conversation, options);
        required.insert(Capability::Streaming);
        ensure_supported(&self.name, &self.model(), &required)?;
        let events: Vec<Result<TurnEvent, ProviderError>> =
            self.events.iter().cloned().map(Ok).collect();
        Ok(Box::pin(stream::iter(events)))
    }

    fn count_cost(&self, _usage: &Usage, _model: &str) -> Cost {
        Cost::zero("USD")
    }
}
