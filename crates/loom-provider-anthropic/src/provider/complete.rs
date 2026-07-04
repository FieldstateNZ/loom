//! The non-streaming request path: translate, negotiate prompt-cache support,
//! and send — the logic behind [`Provider::complete`].
//!
//! [`Provider::complete`]: loom_provider::Provider::complete

use loom_core::{Conversation, ConversationOptions, Message};
use loom_provider::{ensure_supported, required_capabilities, ProviderError};

use crate::catalogue::PROVIDER_NAME;
use crate::translate;

use super::AnthropicProvider;

impl AnthropicProvider {
    /// Executes the non-streaming Messages request: resolves and checks the
    /// model, translates the request, negotiates prompt caching, sends it, and
    /// translates the response.
    pub(super) async fn complete_impl(
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
}
