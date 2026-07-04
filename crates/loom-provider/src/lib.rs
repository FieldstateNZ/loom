//! `loom-provider` — the provider plugin trait, capability model and streaming
//! envelope.
//!
//! Providers are pluggable libraries. Each declares its [capabilities] per
//! model and owns translation between Loom's fluent [`Conversation`] and its
//! native wire protocol; the gateway core never special-cases a provider — it
//! only ever sees the [`Provider`] trait.
//!
//! [`Conversation`]: loom_core::Conversation
//! [capabilities]: Capability
//!
//! # What this crate provides
//!
//! - [`Provider`] — the async plugin trait: [`descriptor`](Provider::descriptor),
//!   [`complete`](Provider::complete), [`stream`](Provider::stream) and the
//!   pricing hook [`count_cost`](Provider::count_cost).
//! - [`Capability`], [`ModelDescriptor`] and [`ProviderDescriptor`] — the
//!   capability model, plus [`required_capabilities`] / [`ensure_supported`]
//!   for **fail-fast** capability negotiation (never silent degradation).
//! - [`TurnEvent`] — the streaming envelope, carrying **both** a normalised
//!   [`TurnEventKind`] and the verbatim native provider event, honouring Loom's
//!   fidelity promise at the streaming level.
//! - [`ProviderRegistry`] — name → provider lookup, designed to admit dynamic
//!   loading later.
//! - [`ProviderError`] — structured errors that preserve provider-native error
//!   payloads.
//!
//! A [`MockProvider`](mock::MockProvider) is available under the `mock` feature
//! for downstream crates' tests.
//!
//! See the [`Provider`] trait docs for a worked example provider
//! implementation sketch.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod capability;
mod error;
mod event;
mod provider;
mod registry;

#[cfg(any(test, feature = "mock"))]
pub mod mock;

pub use capability::{
    ensure_supported, required_capabilities, Capability, ModelDescriptor, ProviderDescriptor,
};
pub use error::{Cost, ProviderError};
pub use event::{ContentDelta, StopReason, TurnEvent, TurnEventKind};
pub use provider::{Provider, TurnEventStream};
pub use registry::ProviderRegistry;

/// Re-export of the domain model every provider translates to and from.
pub use loom_core;

/// The crate version, sourced from Cargo at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use loom_core::{
        ContentPart, Conversation, ConversationOptions, MediaSource, Message, ProviderBinding,
        Role, ToolDefinition,
    };
    use uuid::Uuid;

    use super::mock::MockProvider;
    use super::*;

    fn conversation(model: &str) -> Conversation {
        Conversation::new(Uuid::new_v4(), ProviderBinding::new("mock", model))
    }

    #[test]
    fn required_capabilities_detects_tools_and_content() {
        let mut conv = conversation("m");
        conv.messages.push(Message::new(
            Role::User,
            vec![
                ContentPart::text("look at this"),
                ContentPart::Image {
                    source: MediaSource::Url {
                        url: "https://example.com/a.png".to_owned(),
                    },
                },
            ],
        ));
        let mut opts = ConversationOptions::new();
        opts.tools.push(ToolDefinition {
            name: "get_weather".to_owned(),
            description: None,
            input_schema: serde_json::json!({}),
        });

        let required = required_capabilities(&conv, &opts);
        assert!(required.contains(&Capability::ClientTools));
        assert!(required.contains(&Capability::Vision));
        assert!(!required.contains(&Capability::Documents));
    }

    #[test]
    fn required_capabilities_detects_server_tool_use() {
        let mut conv = conversation("m");
        conv.messages.push(Message::new(
            Role::Assistant,
            vec![ContentPart::ServerToolUse {
                id: "s1".to_owned(),
                name: "web_search".to_owned(),
                input: serde_json::json!({ "query": "loom" }),
            }],
        ));
        let required = required_capabilities(&conv, &ConversationOptions::new());
        assert!(required.contains(&Capability::ServerToolWebSearch));
    }

    #[test]
    fn ensure_supported_passes_when_capabilities_present() {
        let model = ModelDescriptor::new("m", [Capability::ClientTools, Capability::Vision]);
        let mut required = BTreeSet::new();
        required.insert(Capability::ClientTools);
        assert!(ensure_supported("mock", &model, &required).is_ok());
    }

    #[test]
    fn ensure_supported_fails_fast_on_unsupported() {
        let model = ModelDescriptor::new("m", [Capability::ClientTools]);
        let mut required = BTreeSet::new();
        required.insert(Capability::Vision);
        let err = ensure_supported("mock", &model, &required).unwrap_err();
        match err {
            ProviderError::CapabilityUnsupported {
                capability,
                provider,
                model,
            } => {
                assert_eq!(capability, Capability::Vision);
                assert_eq!(provider, "mock");
                assert_eq!(model, "m");
            }
            other => panic!("expected CapabilityUnsupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mock_provider_completes_when_supported() {
        let provider = MockProvider::new("mock", "m", [Capability::ClientTools])
            .with_completion(Message::assistant("canned"));
        let mut conv = conversation("m");
        conv.messages.push(Message::user("hi"));
        let msg = provider
            .complete(&conv, &ConversationOptions::new())
            .await
            .expect("should complete");
        assert_eq!(msg.role, Role::Assistant);
    }

    #[tokio::test]
    async fn mock_provider_rejects_unsupported_request() {
        // Model has no Vision capability, but the request includes an image.
        let provider = MockProvider::new("mock", "m", [Capability::ClientTools]);
        let mut conv = conversation("m");
        conv.messages.push(Message::new(
            Role::User,
            vec![ContentPart::Image {
                source: MediaSource::Url {
                    url: "https://example.com/a.png".to_owned(),
                },
            }],
        ));
        let err = provider
            .complete(&conv, &ConversationOptions::new())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ProviderError::CapabilityUnsupported {
                capability: Capability::Vision,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn mock_provider_streams_events_with_raw() {
        use futures::StreamExt;

        let provider = MockProvider::new("mock", "m", [Capability::Streaming]);
        let conv = conversation("m");
        let stream = provider
            .stream(&conv, &ConversationOptions::new())
            .await
            .expect("should start stream");
        let events: Vec<_> = stream.collect().await;
        assert_eq!(events.len(), 2);
        let first = events[0].as_ref().expect("event ok");
        assert_eq!(first.kind, TurnEventKind::TurnStarted);
        assert!(first.raw.is_object());
    }

    #[tokio::test]
    async fn mock_provider_stream_requires_streaming_capability() {
        // No Streaming capability declared.
        let provider = MockProvider::new("mock", "m", [Capability::ClientTools]);
        let conv = conversation("m");
        let err = provider
            .stream(&conv, &ConversationOptions::new())
            .await
            .err()
            .expect("should reject");
        assert!(matches!(
            err,
            ProviderError::CapabilityUnsupported {
                capability: Capability::Streaming,
                ..
            }
        ));
    }

    #[test]
    fn registry_registers_and_looks_up() {
        let mut registry = ProviderRegistry::new();
        let provider = Arc::new(MockProvider::new("mock", "m", [Capability::Streaming]));
        registry.register(provider);
        assert!(registry.contains("mock"));
        assert_eq!(registry.len(), 1);
        let found = registry.get("mock").expect("registered");
        assert_eq!(found.descriptor().name, "mock");
        assert!(registry.get("nope").is_none());
        assert_eq!(registry.names().collect::<Vec<_>>(), vec!["mock"]);
    }

    #[test]
    fn turn_event_round_trips_through_serde() {
        let mut usage = loom_core::Usage::new();
        usage.output_tokens = Some(7);
        let event = TurnEvent::new(
            TurnEventKind::TurnEnded {
                stop_reason: StopReason::MaxTokens,
                usage: Some(usage),
            },
            serde_json::json!({ "type": "message_delta" }),
        );
        let json = serde_json::to_value(&event).expect("serialize");
        let back: TurnEvent = serde_json::from_value(json).expect("deserialize");
        assert_eq!(event, back);
    }

    #[test]
    fn descriptor_model_lookup() {
        let descriptor =
            ProviderDescriptor::new("mock", [ModelDescriptor::new("m", [Capability::Batches])])
                .with_dynamic_discovery();
        assert!(descriptor.dynamic_discovery);
        assert!(descriptor.model("m").unwrap().supports(Capability::Batches));
        assert!(descriptor.model("absent").is_none());
    }
}
