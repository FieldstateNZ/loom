use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::StreamExt;
use loom_core::{
    CacheNegotiation, ContentPart, Conversation, ConversationOptions, Message, ProviderBinding,
    Role, ToolDefinition,
};
use loom_provider::{Capability, Provider, ProviderError, StopReason, TurnEventKind};
use loom_provider_openai::{translate, OpenAIProvider};
use serde_json::{json, Value};
use uuid::Uuid;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn conversation() -> Conversation {
    let mut conversation =
        Conversation::new(Uuid::new_v4(), ProviderBinding::new("openai", "gpt-6-luna"));
    conversation.system = Some("You are Riley.".into());
    conversation.messages.push(Message::user("Hello"));
    conversation
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/response-text.json")).unwrap()
}
fn tool_response() -> Value {
    json!({"id":"resp_tool","status":"completed","output":[
        {"id":"rs_tool","type":"reasoning","summary":[],"encrypted_content":"opaque-tool-reasoning"},
        {"id":"fc_item","type":"function_call","status":"completed","call_id":"call_memory","name":"memory_recall","arguments":"{ \"query\": \"Riley\" }"}
    ],"usage":{"input_tokens":10,"output_tokens":3}})
}
fn sse(events: &[Value]) -> String {
    events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect()
}
async fn server(template: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer synthetic-key"))
        .respond_with(template)
        .expect(1)
        .mount(&server)
        .await;
    server
}
fn client(server: &MockServer) -> OpenAIProvider {
    OpenAIProvider::new("synthetic-key")
        .unwrap()
        .with_base_url(server.uri())
        .unwrap()
}
async fn stream(body: String) -> Vec<Result<loom_provider::TurnEvent, ProviderError>> {
    let server =
        server(ResponseTemplate::new(200).set_body_raw(body.into_bytes(), "text/event-stream"))
            .await;
    client(&server)
        .stream(&conversation(), &ConversationOptions::new())
        .await
        .unwrap()
        .collect()
        .await
}

#[test]
fn translation_preserves_native_reasoning_message_phase_and_unknown_items_on_replay() {
    let mut native = fixture();
    native["output"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"future_1","type":"future_item","data":{"x":1}}));
    let message = translate::response(&native).unwrap();
    assert_eq!(message.raw.as_ref(), Some(&native));
    let mut convo = conversation();
    convo.messages.push(message);
    convo.messages.push(Message::user("Follow up"));
    let request = translate::request(&convo, &ConversationOptions::new(), false).unwrap();
    assert_eq!(
        &request["input"].as_array().unwrap()[1..4],
        native["output"].as_array().unwrap()
    );
    assert_eq!(request["store"], false);
    assert_eq!(request["instructions"], "You are Riley.");
    assert_eq!(request["reasoning"]["effort"], "none");
    assert!(request.get("previous_response_id").is_none());
    assert_eq!(request["input"][4]["content"][0]["text"], "Follow up");
}

#[test]
fn tool_call_ids_arguments_and_error_results_survive_native_round_trip() {
    let native = tool_response();
    let message = translate::response(&native).unwrap();
    assert_eq!(translate::stop_reason(&native), StopReason::ToolUse);
    let mut convo = conversation();
    convo.messages.push(message);
    convo.messages.push(Message::new(
        Role::User,
        vec![ContentPart::ToolResult {
            tool_use_id: "call_memory".into(),
            content: json!({"error":"denied"}),
            is_error: Some(true),
            cache: None,
        }],
    ));
    let request = translate::request(&convo, &ConversationOptions::new(), false).unwrap();
    assert_eq!(request["input"][2], native["output"][1]);
    assert_eq!(request["input"][3]["call_id"], "call_memory");
    let output: Value =
        serde_json::from_str(request["input"][3]["output"].as_str().unwrap()).unwrap();
    assert_eq!(
        output,
        json!({"is_error":true,"content":{"error":"denied"}})
    );
}

#[test]
fn generic_tool_schemas_keep_optional_fields_and_native_options_cannot_replace_history() {
    let mut options = ConversationOptions::new();
    options.tools.push(ToolDefinition {name:"memory_recall".into(),description:Some("Recall".into()),input_schema:json!({"type":"object","properties":{"query":{"type":"string"}},"required":[]}),cache:None});
    let request = translate::request(&conversation(), &options, false).unwrap();
    assert_eq!(request["tools"][0]["strict"], false);
    assert_eq!(request["tools"][0]["parameters"]["required"], json!([]));
    for field in [
        "input",
        "instructions",
        "store",
        "stream",
        "model",
        "conversation",
        "previous_response_id",
        "background",
        "tools",
        "max_output_tokens",
    ] {
        options
            .provider_options
            .insert("openai".into(), json!({field:"override"}));
        assert!(
            translate::request(&conversation(), &options, false).is_err(),
            "{field} must not override owned state"
        );
    }
}

#[test]
fn usage_splits_cached_input_without_double_charging_and_keeps_native_details() {
    let native = fixture();
    let usage = translate::usage(&native["usage"]);
    assert_eq!(usage.input_tokens, Some(6));
    assert_eq!(usage.cache_read_tokens, Some(4));
    assert_eq!(usage.output_tokens, Some(5));
    assert_eq!(usage.raw.as_ref(), Some(&native["usage"]));
    assert_eq!(
        translate::usage(&json!({"input_tokens":2,"input_tokens_details":{"cached_tokens":4}}))
            .input_tokens,
        Some(0)
    );
}

#[test]
fn base_urls_must_not_leak_credentials_to_redirects_or_untrusted_http() {
    for url in [
        "http://example.com",
        "https://user:password@example.com",
        "https://example.com?key=secret",
        "https://example.com/#fragment",
    ] {
        assert!(OpenAIProvider::new("synthetic-key")
            .unwrap()
            .with_base_url(url)
            .is_err());
    }
    assert!(OpenAIProvider::new("").is_err());
}

#[tokio::test]
async fn nonstreaming_http_uses_responses_and_preserves_the_authoritative_payload() {
    let native = fixture();
    let server = server(ResponseTemplate::new(200).set_body_json(&native)).await;
    let result = client(&server)
        .complete(&conversation(), &ConversationOptions::new())
        .await
        .unwrap();
    assert_eq!(result, translate::response(&native).unwrap());
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "gpt-6-luna");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], false);
}

#[tokio::test]
async fn native_http_errors_are_retained_without_automatic_retries() {
    for status in [401, 429, 500, 503] {
        let native = json!({"error":{"code":"synthetic_failure","message":"native diagnostic"}});
        let server = server(ResponseTemplate::new(status).set_body_json(&native)).await;
        let error = client(&server)
            .complete(&conversation(), &ConversationOptions::new())
            .await
            .unwrap_err();
        assert!(
            matches!(error,ProviderError::Api {status:Some(s),payload:Some(value),..} if s==status && value==native)
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn incomplete_nonstreaming_response_is_never_a_successful_answer() {
    let mut native = fixture();
    native["status"] = json!("incomplete");
    native["incomplete_details"] = json!({"reason":"max_output_tokens"});
    let server = server(ResponseTemplate::new(200).set_body_json(&native)).await;
    assert!(matches!(
        client(&server)
            .complete(&conversation(), &ConversationOptions::new())
            .await,
        Err(ProviderError::Api {
            payload: Some(_),
            ..
        })
    ));
}

#[tokio::test]
async fn capability_negotiation_rejects_unimplemented_mcp_and_documents_before_http() {
    let server = MockServer::start().await;
    let provider = client(&server);
    let mut options = ConversationOptions::new();
    options
        .mcp_servers
        .push(loom_core::McpServerRef::named("lucidbrain"));
    assert!(matches!(
        provider.complete(&conversation(), &options).await,
        Err(ProviderError::CapabilityUnsupported {
            capability: Capability::McpConnector,
            ..
        })
    ));
    let mut convo = conversation();
    convo.binding.model = "unknown-model".into();
    assert!(matches!(
        provider.complete(&convo, &ConversationOptions::new()).await,
        Err(ProviderError::ModelNotFound { .. })
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn strict_cache_markers_and_unsupported_stop_sequences_fail_before_dispatch() {
    let server = MockServer::start().await;
    let provider = client(&server);
    let mut options = ConversationOptions::new();
    options.auto_cache = true;
    options.cache_negotiation = CacheNegotiation::HardFail;
    assert!(matches!(
        provider.complete(&conversation(), &options).await,
        Err(ProviderError::CapabilityUnsupported {
            capability: Capability::PromptCaching,
            ..
        })
    ));
    options = ConversationOptions::new();
    options.stop_sequences.push("stop".into());
    assert!(provider.complete(&conversation(), &options).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn stream_reassembles_to_the_same_content_usage_and_replay_as_nonstreaming() {
    let events = stream(include_str!("fixtures/stream-text.sse").into()).await;
    let mut content = BTreeMap::new();
    let mut usage = None;
    let mut reason = None;
    assert!(matches!(
        events[0].as_ref().unwrap().kind,
        TurnEventKind::TurnStarted
    ));
    for event in events {
        let event = event.unwrap();
        assert!(event.raw["type"].as_str().unwrap().starts_with("response."));
        match event.kind {
            TurnEventKind::ContentPartComplete { index, part } => {
                content.insert(index, part);
            }
            TurnEventKind::TurnEnded {
                usage: final_usage,
                stop_reason,
                ..
            } => {
                usage = final_usage;
                reason = Some(stop_reason);
            }
            _ => {}
        }
    }
    let expected = translate::response(&fixture()).unwrap();
    assert_eq!(content.into_values().collect::<Vec<_>>(), expected.content);
    assert_eq!(usage, expected.usage);
    assert_eq!(reason, Some(StopReason::EndTurn));
}

#[tokio::test]
async fn streaming_function_calls_keep_native_ids_and_stop_for_host_execution() {
    let response = tool_response();
    let call = response["output"][1].clone();
    let mut initial = call.clone();
    initial["arguments"] = json!("");
    initial["status"] = json!("in_progress");
    let events=stream(sse(&[
        json!({"type":"response.created","response":{"id":"resp_tool","status":"in_progress"}}),
        json!({"type":"response.output_item.added","output_index":1,"item":initial}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{ \"query\": \"Riley\" }"}),
        json!({"type":"response.output_item.done","output_index":1,"item":call}),
        json!({"type":"response.completed","response":response})
    ])).await;
    assert!(events.iter().all(Result::is_ok));
    assert!(events.iter().any(|event| matches!(&event.as_ref().unwrap().kind,TurnEventKind::ContentPartComplete {part:ContentPart::ToolUse {id,input,..},..} if id=="call_memory" && input==&json!({"query":"Riley"}))));
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap().kind,
        TurnEventKind::TurnEnded {
            stop_reason: StopReason::ToolUse,
            ..
        }
    ));
}

#[tokio::test]
async fn early_eof_malformed_json_and_native_failure_never_emit_successful_terminal_events() {
    let prefix =
        "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{}}\n\n";
    for body in [prefix.to_owned(),format!("{prefix}data: invalid json\n\n"),format!("{prefix}data: {{\"type\":\"response.failed\",\"response\":{{\"status\":\"failed\"}}}}\n\n")] {
        let events=stream(body).await;
        assert!(events.last().unwrap().is_err());
        assert!(!events.iter().filter_map(|e|e.as_ref().ok()).any(|event|matches!(event.kind,TurnEventKind::TurnEnded {..})));
    }
}

#[tokio::test]
async fn streaming_incomplete_status_is_exposed_as_max_tokens_not_end_turn() {
    let mut response = fixture();
    response["status"] = json!("incomplete");
    response["incomplete_details"] = json!({"reason":"max_output_tokens"});
    let events = stream(sse(&[
        json!({"type":"response.created","response":{}}),
        json!({"type":"response.incomplete","response":response}),
    ]))
    .await;
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap().kind,
        TurnEventKind::TurnEnded {
            stop_reason: StopReason::MaxTokens,
            ..
        }
    ));
}

#[tokio::test]
async fn request_cancellation_drops_the_inflight_call_without_retrying() {
    let server = server(
        ResponseTemplate::new(200)
            .set_body_json(fixture())
            .set_delay(Duration::from_secs(2)),
    )
    .await;
    let provider = client(&server);
    let convo = conversation();
    let options = ConversationOptions::new();
    assert!(tokio::time::timeout(
        Duration::from_millis(100),
        provider.complete(&convo, &options)
    )
    .await
    .is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn parallel_output_is_normalized_in_native_item_order() {
    let message = json!({"id":"msg_parallel","type":"message","role":"assistant","status":"completed","phase":"commentary","content":[{"type":"output_text","text":"Checking memory.","annotations":[]}]});
    let call = tool_response()["output"][1].clone();
    let mut initial_call = call.clone();
    initial_call["arguments"] = json!("");
    let mut initial_message = message.clone();
    initial_message["content"] = json!([]);
    initial_message["status"] = json!("in_progress");
    let native = json!({"id":"resp_parallel","status":"completed","output":[message,call]});
    let events=stream(sse(&[
        json!({"type":"response.created","response":{}}),
        json!({"type":"response.output_item.added","output_index":0,"item":initial_message}),
        json!({"type":"response.output_item.added","output_index":1,"item":initial_call}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{ \"query\": \"Riley\" }"}),
        json!({"type":"response.content_part.added","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"Checking memory."}),
        json!({"type":"response.output_item.done","output_index":0,"item":native["output"][0]}),
        json!({"type":"response.output_item.done","output_index":1,"item":native["output"][1]}),
        json!({"type":"response.completed","response":native})
    ])).await;
    let mut parts = BTreeMap::new();
    for event in events {
        if let TurnEventKind::ContentPartComplete { index, part } = event.unwrap().kind {
            parts.insert(index, part);
        }
    }
    assert_eq!(
        parts.into_values().collect::<Vec<_>>(),
        translate::response(&native).unwrap().content
    );
}

#[tokio::test]
async fn a_redirect_is_an_error_without_sending_the_credential_onward() {
    let server = server(
        ResponseTemplate::new(302)
            .insert_header("location", "https://redirect.example.invalid/v1/responses"),
    )
    .await;
    assert!(matches!(
        client(&server)
            .complete(&conversation(), &ConversationOptions::new())
            .await,
        Err(ProviderError::Api {
            status: Some(302),
            ..
        })
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn refusal_content_is_preserved_and_has_a_distinct_terminal_reason() {
    let native = json!({"id":"resp_refusal","status":"completed","output":[{"id":"msg_refusal","type":"message","role":"assistant","status":"completed","content":[{"type":"refusal","refusal":"I cannot do that."}]}]});
    let events = stream(sse(&[
        json!({"type":"response.created","response":{}}),
        json!({"type":"response.completed","response":native}),
    ]))
    .await;
    assert!(events.iter().any(|event|matches!(&event.as_ref().unwrap().kind,TurnEventKind::ContentPartComplete {part:ContentPart::ProviderExtension {kind,payload,..},..} if kind=="refusal" && payload["refusal"]=="I cannot do that.")));
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap().kind,
        TurnEventKind::TurnEnded {
            stop_reason: StopReason::Refusal,
            ..
        }
    ));
}
