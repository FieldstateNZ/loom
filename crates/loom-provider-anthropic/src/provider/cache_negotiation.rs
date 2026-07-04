//! Prompt-cache capability negotiation applied to an already-built request
//! body before dispatch.

use loom_core::{CacheNegotiation, Conversation, ConversationOptions};
use loom_provider::{Capability, ModelDescriptor, ProviderError};
use serde_json::Value;

use crate::catalogue::PROVIDER_NAME;
use crate::translate;

use super::AnthropicProvider;

impl AnthropicProvider {
    /// Applies prompt-cache negotiation to an already-built request `body`.
    ///
    /// Cache hints are advisory: if the request carries any cache directive but
    /// the bound `model` does not declare [`Capability::PromptCaching`], the
    /// [`ConversationOptions::cache_negotiation`] policy decides the outcome —
    /// [`CacheNegotiation::SoftIgnore`] (the default) strips the `cache_control`
    /// markers and logs a warning, while [`CacheNegotiation::HardFail`] returns
    /// [`ProviderError::CapabilityUnsupported`]. When the model supports
    /// caching, the body is returned unchanged.
    pub(super) fn negotiate_cache(
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
