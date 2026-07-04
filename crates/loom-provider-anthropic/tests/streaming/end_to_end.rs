//! End-to-end path: [`Provider::stream`] driven over a real HTTP + SSE
//! response from a `wiremock` server.

use futures::StreamExt;
use loom_core::{Conversation, ConversationOptions, ProviderBinding};
use loom_provider::{Provider, ProviderError, TurnEventKind};
use loom_provider_anthropic::{translate, AnthropicProvider, SseAccumulator};
use serde_json::Value;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn conversation() -> Conversation {
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.messages.push(loom_core::Message::user("hi"));
    conversation
}

async fn sse_server(body: &'static str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn stream_end_to_end_parses_and_reassembles_over_http() {
    let server = sse_server(include_str!("../fixtures/stream_tool_use.sse")).await;
    let provider = AnthropicProvider::new("test-key")
        .expect("build provider")
        .with_base_url(server.uri());

    let stream = provider
        .stream(&conversation(), &ConversationOptions::new())
        .await
        .expect("stream opens");

    // Fold the stream through a caller-side accumulator — the documented way to
    // reconstruct the turn — while collecting the normalised kinds.
    let mut accumulator = SseAccumulator::new();
    let mut kinds = Vec::new();
    let mut items = stream;
    while let Some(item) = items.next().await {
        let event = item.expect("no mid-stream error");
        kinds.push(event.kind.clone());
        accumulator.ingest(event.raw).expect("ingest ok");
    }

    assert_eq!(kinds.first(), Some(&TurnEventKind::TurnStarted));
    assert!(matches!(
        kinds.last(),
        Some(TurnEventKind::Other { native_type }) if native_type.as_deref() == Some("message_stop")
    ));

    let non_streaming: Value =
        serde_json::from_str(include_str!("../fixtures/stream_tool_use_response.json"))
            .expect("valid fixture");
    assert_eq!(
        accumulator.message(),
        translate::translate_response(&non_streaming),
        "reassembled over HTTP must equal the non-streaming message"
    );
}

#[tokio::test]
async fn stream_end_to_end_surfaces_mid_stream_error_with_partial() {
    let server = sse_server(include_str!("../fixtures/stream_error.sse")).await;
    let provider = AnthropicProvider::new("test-key")
        .expect("build provider")
        .with_base_url(server.uri());

    let stream = provider
        .stream(&conversation(), &ConversationOptions::new())
        .await
        .expect("stream opens");

    let mut accumulator = SseAccumulator::new();
    let mut error = None;
    let mut items = stream;
    while let Some(item) = items.next().await {
        match item {
            Ok(event) => {
                accumulator.ingest(event.raw).expect("ingest ok");
            }
            Err(err) => {
                error = Some(err);
                break;
            }
        }
    }

    // The error surfaced, and the partial turn assembled before it survives.
    match error.expect("stream yielded an error") {
        ProviderError::Api { message, .. } => assert_eq!(message, "Overloaded"),
        other => panic!("expected Api error, got {other:?}"),
    }
    assert_eq!(
        accumulator.message().content[0],
        loom_core::ContentPart::text("Partial answer")
    );
}

#[tokio::test]
async fn stream_rejects_a_model_without_streaming_before_any_request() {
    let server = MockServer::start().await;
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "no-such-model"),
    );
    conversation.messages.push(loom_core::Message::user("hi"));

    let error = AnthropicProvider::new("k")
        .expect("build provider")
        .with_base_url(server.uri())
        .stream(&conversation, &ConversationOptions::new())
        .await
        .err()
        .expect("unknown model rejected before any HTTP call");
    assert!(matches!(error, ProviderError::ModelNotFound { .. }));
}
