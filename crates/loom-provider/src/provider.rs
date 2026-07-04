//! The [`Provider`] plugin trait.

use async_trait::async_trait;
use futures::stream::BoxStream;
use loom_core::{Conversation, ConversationOptions, Message, Usage};

use crate::capability::ProviderDescriptor;
use crate::error::{Cost, ProviderError};
use crate::event::TurnEvent;

/// A stream of streaming turn events, boxed so the trait stays object-safe.
///
/// Each item is a `Result` so a mid-stream provider failure can be surfaced
/// without tearing down the whole stream.
pub type TurnEventStream = BoxStream<'static, Result<TurnEvent, ProviderError>>;

/// A pluggable LLM provider.
///
/// A provider owns translation between Loom's fluent [`Conversation`] model and
/// its own native wire protocol, declares its [capabilities] per model, and
/// exposes both a non-streaming and a streaming completion path. The gateway
/// core never special-cases a concrete provider — it only ever sees this trait.
///
/// Implementations should perform [capability negotiation] before dispatching a
/// request, failing fast with [`ProviderError::CapabilityUnsupported`] rather
/// than silently dropping a feature the caller asked for.
///
/// [capabilities]: crate::Capability
/// [capability negotiation]: crate::ensure_supported
///
/// # Implementing a provider
///
/// The sketch below shows the shape of a real implementation (translation
/// bodies elided). A production provider would build a native request from the
/// conversation, call its HTTP API, and translate the response back — all
/// without flattening provider-specific concepts.
///
/// ```
/// use async_trait::async_trait;
/// use futures::stream;
/// use loom_core::{Conversation, ConversationOptions, Message, Role, Usage};
/// use loom_provider::{
///     ensure_supported, required_capabilities, Capability, Cost, ModelDescriptor, Provider,
///     ProviderDescriptor, ProviderError, TurnEvent, TurnEventKind, TurnEventStream,
/// };
///
/// struct AcmeProvider;
///
/// impl AcmeProvider {
///     fn model(&self, id: &str) -> Result<ModelDescriptor, ProviderError> {
///         self.descriptor()
///             .models
///             .into_iter()
///             .find(|m| m.id == id)
///             .ok_or_else(|| ProviderError::ModelNotFound {
///                 provider: "acme".to_owned(),
///                 model: id.to_owned(),
///             })
///     }
/// }
///
/// #[async_trait]
/// impl Provider for AcmeProvider {
///     fn descriptor(&self) -> ProviderDescriptor {
///         ProviderDescriptor::new(
///             "acme",
///             [ModelDescriptor::new(
///                 "acme-large",
///                 [Capability::Streaming, Capability::ClientTools],
///             )],
///         )
///     }
///
///     async fn complete(
///         &self,
///         conversation: &Conversation,
///         options: &ConversationOptions,
///     ) -> Result<Message, ProviderError> {
///         let model = self.model(&conversation.binding.model)?;
///         ensure_supported("acme", &model, &required_capabilities(conversation, options))?;
///         // ... translate → call native API → translate response back ...
///         Ok(Message::assistant("hi"))
///     }
///
///     async fn stream(
///         &self,
///         conversation: &Conversation,
///         options: &ConversationOptions,
///     ) -> Result<TurnEventStream, ProviderError> {
///         let model = self.model(&conversation.binding.model)?;
///         let mut required = required_capabilities(conversation, options);
///         required.insert(Capability::Streaming);
///         ensure_supported("acme", &model, &required)?;
///         let events = vec![Ok(TurnEvent::new(
///             TurnEventKind::TurnStarted,
///             serde_json::json!({ "type": "message_start" }),
///         ))];
///         Ok(Box::pin(stream::iter(events)))
///     }
///
///     fn count_cost(&self, _usage: &Usage, _model: &str) -> Cost {
///         // Pricing data lands separately; this hook returns a placeholder.
///         Cost::zero("USD")
///     }
/// }
/// ```
#[async_trait]
pub trait Provider: Send + Sync {
    /// Describes this provider: its name and the models (with capabilities) it
    /// offers.
    fn descriptor(&self) -> ProviderDescriptor;

    /// Runs a non-streaming completion, returning the assistant [`Message`].
    ///
    /// The returned message must preserve the provider's native response
    /// faithfully, including a raw payload for anything Loom does not model as a
    /// typed field.
    async fn complete(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
    ) -> Result<Message, ProviderError>;

    /// Runs a streaming completion, returning a stream of [`TurnEvent`]s.
    ///
    /// Every event carries both the normalised envelope and the verbatim native
    /// provider event, so consumers may work at either level.
    async fn stream(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
    ) -> Result<TurnEventStream, ProviderError>;

    /// Pricing **hook**: computes the [`Cost`] of `usage` for `model`.
    ///
    /// This trait supplies only the hook; the pricing *data* (per-model rates)
    /// is provided by the spend-tracking layer. Implementations without rate
    /// data should return [`Cost::zero`].
    fn count_cost(&self, usage: &Usage, model: &str) -> Cost;
}
