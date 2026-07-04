//! The streaming request path: translate, negotiate prompt-cache support, send
//! with `"stream": true`, and hand the response to the SSE plumbing — the
//! logic behind [`Provider::stream`].
//!
//! [`Provider::stream`]: loom_provider::Provider::stream

use loom_core::{Conversation, ConversationOptions};
use loom_provider::{
    ensure_supported, required_capabilities, Capability, ProviderError, TurnEventStream,
};
use serde_json::{json, Value};

use crate::catalogue::PROVIDER_NAME;
use crate::translate;

use super::AnthropicProvider;

impl AnthropicProvider {
    /// Executes the streaming Messages request: resolves and checks the model
    /// (requiring [`Capability::Streaming`] in addition to the request's other
    /// capabilities), translates the request, negotiates prompt caching, marks
    /// it as a streaming request, sends it, and hands the response to
    /// [`crate::streaming::event_stream`].
    pub(super) async fn stream_impl(
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
        Ok(crate::streaming::event_stream(response))
    }
}
