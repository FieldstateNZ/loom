//! [`AnthropicProvider`]'s [`Provider`] trait implementation.
//!
//! The non-streaming and streaming bodies live in the sibling [`complete`] and
//! [`stream`] submodules respectively; this block wires them to the trait.
//!
//! [`Provider`]: loom_provider::Provider
//! [`complete`]: super::complete
//! [`stream`]: super::stream

use async_trait::async_trait;
use loom_core::{Conversation, ConversationOptions, Message, Usage};
use loom_provider::{Cost, Provider, ProviderDescriptor, ProviderError, TurnEventStream};

use crate::catalogue::{catalogue, PROVIDER_NAME};

use super::AnthropicProvider;

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
        self.complete_impl(conversation, options).await
    }

    async fn stream(
        &self,
        conversation: &Conversation,
        options: &ConversationOptions,
    ) -> Result<TurnEventStream, ProviderError> {
        self.stream_impl(conversation, options).await
    }

    fn count_cost(&self, _usage: &Usage, _model: &str) -> Cost {
        // Pricing data lands with issue #9; the hook returns a placeholder.
        Cost::zero("USD")
    }
}
