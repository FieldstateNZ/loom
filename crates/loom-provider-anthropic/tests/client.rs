//! End-to-end HTTP client tests for the Anthropic provider, driven against a
//! `wiremock` server rather than a live API.
//!
//! Covers the request headers, the happy path, retry-with-backoff on a
//! transient `5xx`, and error mapping on `4xx`/`5xx`.

use std::time::Duration;

use loom_core::{Conversation, ConversationOptions, Message, ProviderBinding, Role};
use loom_provider::{Provider, ProviderError};
use loom_provider_anthropic::AnthropicProvider;
use serde_json::{json, Value};
use uuid::Uuid;
use wiremock::matchers::{body_json_string, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn response_fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/messages_response.json")).expect("valid fixture")
}

fn conversation() -> Conversation {
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.messages.push(Message::user("hi"));
    conversation
}

fn provider(server: &MockServer) -> AnthropicProvider {
    AnthropicProvider::new("test-key")
        .expect("build provider")
        .with_base_url(server.uri())
        .with_max_retries(2)
        .with_retry_base_delay(Duration::from_millis(1))
}

#[tokio::test]
async fn complete_sends_headers_and_maps_the_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_fixture()))
        .expect(1)
        .mount(&server)
        .await;

    let message = provider(&server)
        .complete(&conversation(), &ConversationOptions::new())
        .await
        .expect("completes");

    assert_eq!(message.role, Role::Assistant);
    assert_eq!(message.content.len(), 7);
    assert_eq!(message.usage.as_ref().unwrap().output_tokens, Some(128));
    // The verbatim native response is preserved for audit.
    assert_eq!(
        message.raw.as_ref().unwrap()["id"],
        json!("msg_01XyZabc123")
    );
}

#[tokio::test]
async fn complete_sends_the_translated_request_body() {
    let server = MockServer::start().await;

    let mut options = ConversationOptions::new();
    options.max_tokens = Some(64);
    let expected_body =
        loom_provider_anthropic::translate::translate_request(&conversation(), &options);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_json_string(
            serde_json::to_string(&expected_body).unwrap(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_fixture()))
        .expect(1)
        .mount(&server)
        .await;

    provider(&server)
        .complete(&conversation(), &options)
        .await
        .expect("completes");
}

#[tokio::test]
async fn server_tool_feature_adds_the_catalogue_driven_beta_header() {
    let server = MockServer::start().await;

    // A code-execution server tool drives the `anthropic-beta` header, keyed off
    // the catalogue's feature→beta mapping. The mock only matches when that
    // header is present with the expected token.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("anthropic-beta", "code-execution-2025-05-22"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_fixture()))
        .expect(1)
        .mount(&server)
        .await;

    let mut options = ConversationOptions::new();
    options.server_tools = vec![loom_core::ServerTool::CodeExecution {}];

    provider(&server)
        .complete(&conversation(), &options)
        .await
        .expect("code-execution request carries its beta header");
}

#[tokio::test]
async fn configured_beta_is_added_without_a_release() {
    let server = MockServer::start().await;

    // An operator can adopt a new beta via provider config, with no server tool
    // and no request-shape change.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("anthropic-beta", "experimental-2027-01-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_fixture()))
        .expect(1)
        .mount(&server)
        .await;

    AnthropicProvider::new("test-key")
        .expect("build provider")
        .with_base_url(server.uri())
        .with_beta("experimental-2027-01-01")
        .complete(&conversation(), &ConversationOptions::new())
        .await
        .expect("configured beta rides the header");
}

#[tokio::test]
async fn retries_on_transient_5xx_then_succeeds() {
    let server = MockServer::start().await;

    // First request: a transient 503 (consumed after one match).
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;

    // Retry: succeeds.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_fixture()))
        .expect(1)
        .mount(&server)
        .await;

    let message = provider(&server)
        .complete(&conversation(), &ConversationOptions::new())
        .await
        .expect("succeeds after retry");
    assert_eq!(message.role, Role::Assistant);
}

#[tokio::test]
async fn maps_4xx_error_envelope_with_native_payload() {
    let server = MockServer::start().await;
    let error_body: Value =
        serde_json::from_str(include_str!("fixtures/error_response.json")).expect("valid fixture");

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_json(error_body.clone()))
        .mount(&server)
        .await;

    let error = provider(&server)
        .complete(&conversation(), &ConversationOptions::new())
        .await
        .expect_err("400 is an error");

    match error {
        ProviderError::Api {
            status,
            message,
            payload,
        } => {
            assert_eq!(status, Some(400));
            assert_eq!(message, "max_tokens: must be greater than 0");
            // The native error envelope is preserved verbatim.
            assert_eq!(payload, Some(error_body));
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn maps_5xx_error_after_retries_are_exhausted() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "type": "error",
            "error": { "type": "api_error", "message": "internal" }
        })))
        .mount(&server)
        .await;

    let error = provider(&server)
        .complete(&conversation(), &ConversationOptions::new())
        .await
        .expect_err("persistent 500 is an error");

    match error {
        ProviderError::Api { status, .. } => assert_eq!(status, Some(500)),
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn unknown_model_fails_fast_without_a_request() {
    let server = MockServer::start().await;
    // No mock mounted: a request would 404 and surface differently. We assert
    // the model is rejected before any HTTP call.
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "no-such-model"),
    );
    conversation.messages.push(Message::user("hi"));

    let error = provider(&server)
        .complete(&conversation, &ConversationOptions::new())
        .await
        .expect_err("unknown model rejected");
    assert!(matches!(error, ProviderError::ModelNotFound { .. }));
}

/// Live smoke test against the real Anthropic API. Ignored by default; run with
/// `LOOM_ANTHROPIC_LIVE=1` and a valid `ANTHROPIC_API_KEY`.
#[tokio::test]
#[ignore = "requires LOOM_ANTHROPIC_LIVE=1 and ANTHROPIC_API_KEY"]
async fn live_completion() {
    if std::env::var("LOOM_ANTHROPIC_LIVE").as_deref() != Ok("1") {
        return;
    }
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY set for live test");
    let provider = AnthropicProvider::new(api_key).expect("build provider");

    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-haiku-4-5-20251001"),
    );
    conversation
        .messages
        .push(Message::user("Reply with the single word: hello"));
    let mut options = ConversationOptions::new();
    options.max_tokens = Some(16);

    let message = provider
        .complete(&conversation, &options)
        .await
        .expect("live completion succeeds");
    assert_eq!(message.role, Role::Assistant);
    assert!(message.raw.is_some());
}
