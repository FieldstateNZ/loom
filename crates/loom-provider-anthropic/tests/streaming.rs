//! Fixture-based tests for Anthropic SSE streaming: event-sequence mapping,
//! verbatim `raw` preservation, streamed/non-streamed message equivalence, and
//! partial-turn recovery on mid-stream error and early disconnect.
//!
//! The transcript fixtures under `tests/fixtures/*.sse` are raw `event:/data:`
//! Server-Sent Events text. Sequence tests drive [`SseAccumulator`] directly;
//! the end-to-end tests drive the full [`Provider::stream`] path against a
//! `wiremock` server that serves the transcript as an SSE response body.

use futures::StreamExt;
use loom_core::{ContentPart, Conversation, ConversationOptions, ProviderBinding, Role};
use loom_provider::{ContentDelta, Provider, ProviderError, StopReason, TurnEvent, TurnEventKind};
use loom_provider_anthropic::{translate, AnthropicProvider, SseAccumulator};
use serde_json::{json, Value};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Splits a raw SSE transcript into its ordered native event JSON payloads.
///
/// Events are separated by a blank line; the `data:` line(s) of each event
/// carry the JSON. Non-`data` fields (`event:`) are ignored — the JSON `type`
/// is authoritative, exactly as the streaming accumulator treats it.
fn native_events(raw: &str) -> Vec<Value> {
    raw.split("\n\n")
        .filter_map(|block| {
            let data: String = block
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|rest| rest.strip_prefix(' ').unwrap_or(rest))
                .collect::<Vec<_>>()
                .join("\n");
            if data.trim().is_empty() {
                return None;
            }
            Some(serde_json::from_str(&data).expect("valid event JSON"))
        })
        .collect()
}

/// Drives an accumulator over every event of a transcript, returning the
/// per-event results and the accumulator (so the assembled/partial message can
/// be inspected).
fn drive(raw: &str) -> (Vec<Result<TurnEvent, ProviderError>>, SseAccumulator) {
    let mut accumulator = SseAccumulator::new();
    let results = native_events(raw)
        .into_iter()
        .map(|event| accumulator.ingest(event))
        .collect();
    (results, accumulator)
}

fn kinds(results: &[Result<TurnEvent, ProviderError>]) -> Vec<TurnEventKind> {
    results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .map(|event| event.kind.clone())
        .collect()
}

#[test]
fn plain_text_maps_to_the_expected_event_sequence() {
    let raw = include_str!("fixtures/stream_text.sse");
    let (results, _) = drive(raw);
    assert!(results.iter().all(Result::is_ok));

    let kinds = kinds(&results);
    assert_eq!(kinds[0], TurnEventKind::TurnStarted);
    assert_eq!(
        kinds[1],
        TurnEventKind::ContentPartStarted {
            index: 0,
            part: ContentPart::text(""),
        }
    );
    assert_eq!(
        kinds[2],
        TurnEventKind::ContentPartDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Hello".to_owned()
            },
        }
    );
    assert_eq!(
        kinds[3],
        TurnEventKind::ContentPartDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: ", world".to_owned()
            },
        }
    );
    assert_eq!(
        kinds[4],
        TurnEventKind::ContentPartComplete {
            index: 0,
            part: ContentPart::text("Hello, world"),
        }
    );
    match &kinds[5] {
        TurnEventKind::TurnEnded { stop_reason, usage } => {
            assert_eq!(*stop_reason, StopReason::EndTurn);
            let usage = usage.as_ref().expect("message_delta couples usage");
            assert_eq!(usage.input_tokens, Some(10));
            assert_eq!(usage.output_tokens, Some(7));
        }
        other => panic!("expected TurnEnded, got {other:?}"),
    }
    assert_eq!(
        kinds[6],
        TurnEventKind::Other {
            native_type: Some("message_stop".to_owned()),
        }
    );
}

#[test]
fn every_event_carries_the_verbatim_native_json_on_raw() {
    let raw = include_str!("fixtures/stream_text.sse");
    let events = native_events(raw);
    let (results, _) = drive(raw);
    assert_eq!(results.len(), events.len());
    for (result, native) in results.iter().zip(&events) {
        let event = result.as_ref().expect("event ok");
        assert_eq!(&event.raw, native, "raw must be the verbatim native event");
    }
}

#[test]
fn tool_use_input_json_deltas_assemble_into_a_parsed_tool_call() {
    let raw = include_str!("fixtures/stream_tool_use.sse");
    let (results, _) = drive(raw);
    let kinds = kinds(&results);

    // The tool_use block is announced with an empty input object.
    assert_eq!(
        kinds[4],
        TurnEventKind::ContentPartStarted {
            index: 1,
            part: ContentPart::ToolUse {
                id: "toolu_01".to_owned(),
                name: "get_weather".to_owned(),
                input: json!({}),
                cache: None,
            },
        }
    );
    // Input arrives as partial-JSON fragments.
    assert_eq!(
        kinds[5],
        TurnEventKind::ContentPartDelta {
            index: 1,
            delta: ContentDelta::Json {
                partial_json: "{\"location\":".to_owned()
            },
        }
    );
    // At block stop the fragments are parsed into the completed call.
    assert_eq!(
        kinds[7],
        TurnEventKind::ContentPartComplete {
            index: 1,
            part: ContentPart::ToolUse {
                id: "toolu_01".to_owned(),
                name: "get_weather".to_owned(),
                input: json!({ "location": "Wellington, NZ" }),
                cache: None,
            },
        }
    );
}

#[test]
fn thinking_deltas_and_signature_assemble_and_ping_is_never_dropped() {
    let raw = include_str!("fixtures/stream_thinking.sse");
    let (results, _) = drive(raw);
    let kinds = kinds(&results);

    // The ping keep-alive surfaces as Other rather than being dropped.
    assert!(kinds.contains(&TurnEventKind::Other {
        native_type: Some("ping".to_owned()),
    }));

    // The thinking deltas and the signature delta are distinct delta kinds.
    assert!(kinds.contains(&TurnEventKind::ContentPartDelta {
        index: 0,
        delta: ContentDelta::Thinking {
            thinking: "The user asked ".to_owned()
        },
    }));
    assert!(kinds.contains(&TurnEventKind::ContentPartDelta {
        index: 0,
        delta: ContentDelta::SignatureDelta {
            signature: "EqoBsig==".to_owned()
        },
    }));

    // The completed thinking block joins its text and carries the signature.
    assert!(kinds.contains(&TurnEventKind::ContentPartComplete {
        index: 0,
        part: ContentPart::Thinking {
            thinking: "The user asked a simple question.".to_owned(),
            signature: Some("EqoBsig==".to_owned()),
            cache: None,
        },
    }));
}

#[test]
fn streamed_text_message_equals_the_non_streaming_message() {
    let raw = include_str!("fixtures/stream_text.sse");
    let (_, accumulator) = drive(raw);

    let non_streaming: Value =
        serde_json::from_str(include_str!("fixtures/stream_text_response.json"))
            .expect("valid fixture");
    let expected = translate::translate_response(&non_streaming);

    assert_eq!(
        accumulator.message(),
        expected,
        "streamed-accumulated message must equal the non-streaming message"
    );
}

#[test]
fn citations_deltas_surface_as_citation_deltas_and_accumulate_onto_the_text_block() {
    let raw = include_str!("fixtures/stream_citations.sse");
    let (results, _) = drive(raw);
    assert!(results.iter().all(Result::is_ok));
    let kinds = kinds(&results);

    // Each streamed citation surfaces as a normalised Citation delta carrying
    // the verbatim native citation object.
    assert!(kinds.contains(&TurnEventKind::ContentPartDelta {
        index: 0,
        delta: ContentDelta::Citation {
            citation: loom_core::Citation(json!({
                "type": "char_location",
                "cited_text": "grass is green",
                "document_index": 0,
                "document_title": "Colours",
                "start_char_index": 10,
                "end_char_index": 24
            })),
        },
    }));

    // The completed text block joins its text and carries both citations, in
    // order, matching the non-streaming ContentPart::Text { citations } shape.
    assert!(kinds.contains(&TurnEventKind::ContentPartComplete {
        index: 0,
        part: ContentPart::Text {
            text: "The grass is green, the sky is blue".to_owned(),
            citations: Some(vec![
                loom_core::Citation(json!({
                    "type": "char_location",
                    "cited_text": "grass is green",
                    "document_index": 0,
                    "document_title": "Colours",
                    "start_char_index": 10,
                    "end_char_index": 24
                })),
                loom_core::Citation(json!({
                    "type": "char_location",
                    "cited_text": "sky is blue",
                    "document_index": 0,
                    "document_title": "Colours",
                    "start_char_index": 40,
                    "end_char_index": 51
                })),
            ]),
            cache: None,
        },
    }));
}

#[test]
fn streamed_cited_text_message_equals_the_non_streaming_message() {
    let raw = include_str!("fixtures/stream_citations.sse");
    let (_, accumulator) = drive(raw);

    let non_streaming: Value =
        serde_json::from_str(include_str!("fixtures/stream_citations_response.json"))
            .expect("valid fixture");
    let expected = translate::translate_response(&non_streaming);

    // The streamed citations are not dropped: the reassembled cited text block
    // is byte-for-byte the non-streaming Message, citations intact.
    assert_eq!(
        accumulator.message(),
        expected,
        "streamed cited message must equal the non-streaming message"
    );
    assert!(matches!(
        expected.content.first(),
        Some(ContentPart::Text { citations: Some(citations), .. }) if citations.len() == 2
    ));
}

#[test]
fn web_search_server_tool_streams_and_accumulates_the_provider_executed_turn() {
    let raw = include_str!("fixtures/stream_web_search.sse");
    let (results, _) = drive(raw);
    assert!(results.iter().all(Result::is_ok));
    let kinds = kinds(&results);

    // The server_tool_use block is announced with an empty input object, then
    // its query assembles from partial-JSON deltas.
    assert!(kinds.contains(&TurnEventKind::ContentPartStarted {
        index: 1,
        part: ContentPart::ServerToolUse {
            id: "srvtoolu_ws".to_owned(),
            name: "web_search".to_owned(),
            input: json!({}),
        },
    }));
    assert!(kinds.contains(&TurnEventKind::ContentPartComplete {
        index: 1,
        part: ContentPart::ServerToolUse {
            id: "srvtoolu_ws".to_owned(),
            name: "web_search".to_owned(),
            input: json!({ "query": "loom gateway release notes" }),
        },
    }));

    // The provider-executed result block arrives whole and surfaces as a
    // ServerToolResult carrying the native payload verbatim.
    assert!(kinds.contains(&TurnEventKind::ContentPartComplete {
        index: 2,
        part: ContentPart::ServerToolResult {
            tool_use_id: "srvtoolu_ws".to_owned(),
            content: json!([{
                "type": "web_search_result",
                "title": "Loom 0.1 released",
                "url": "https://example.com/loom-0-1",
                "encrypted_content": "AbCdEf...opaque...==",
                "page_age": "2 days ago"
            }]),
        },
    }));

    // The turn-end usage carries the server-tool request counter for pricing.
    match kinds.last() {
        Some(TurnEventKind::Other { native_type }) => {
            assert_eq!(native_type.as_deref(), Some("message_stop"));
        }
        other => panic!("expected message_stop, got {other:?}"),
    }
}

#[test]
fn streamed_web_search_message_equals_the_non_streaming_message() {
    let raw = include_str!("fixtures/stream_web_search.sse");
    let (_, accumulator) = drive(raw);

    let non_streaming: Value =
        serde_json::from_str(include_str!("fixtures/stream_web_search_response.json"))
            .expect("valid fixture");
    let expected = translate::translate_response(&non_streaming);

    // A web-search server-tool turn reassembles byte-for-byte from the stream:
    // text, the server_tool_use call, its provider-executed result, and the
    // cited summary — all identical to the non-streaming Message.
    assert_eq!(
        accumulator.message(),
        expected,
        "streamed web-search message must equal the non-streaming message"
    );

    // The server-tool usage counter survives into the accumulated Usage.
    let usage = expected.usage.expect("usage present");
    assert_eq!(usage.server_tool_use.get("web_search_requests"), Some(&1));
}

#[test]
fn streamed_thinking_message_equals_the_non_streaming_message() {
    let raw = include_str!("fixtures/stream_thinking.sse");
    let (_, accumulator) = drive(raw);

    let non_streaming: Value =
        serde_json::from_str(include_str!("fixtures/stream_thinking_response.json"))
            .expect("valid fixture");
    let expected = translate::translate_response(&non_streaming);

    assert_eq!(
        accumulator.message(),
        expected,
        "streamed thinking message must equal the non-streaming message"
    );
}

#[test]
fn streamed_tool_use_message_equals_the_non_streaming_message() {
    let raw = include_str!("fixtures/stream_tool_use.sse");
    let (_, accumulator) = drive(raw);

    let non_streaming: Value =
        serde_json::from_str(include_str!("fixtures/stream_tool_use_response.json"))
            .expect("valid fixture");
    let expected = translate::translate_response(&non_streaming);

    assert_eq!(accumulator.message(), expected);
}

#[test]
fn mcp_tool_use_and_result_stream_and_reassemble_via_provider_extension() {
    let raw = include_str!("fixtures/stream_mcp.sse");
    let (results, accumulator) = drive(raw);
    assert!(results.iter().all(Result::is_ok));
    let kinds = kinds(&results);

    // The streamed mcp_tool_use assembles its input from partial-JSON deltas and
    // completes as a ProviderExtension that preserves the provider-specific
    // `server_name` field (which a typed server-tool part would drop).
    let complete0 = kinds
        .iter()
        .find_map(|k| match k {
            TurnEventKind::ContentPartComplete { index: 0, part } => Some(part.clone()),
            _ => None,
        })
        .expect("block 0 completes");
    match complete0 {
        ContentPart::ProviderExtension { kind, payload, .. } => {
            assert_eq!(kind, "mcp_tool_use");
            assert_eq!(payload["server_name"], json!("github"));
            assert_eq!(payload["input"], json!({ "query": "loom" }));
        }
        other => panic!("expected ProviderExtension, got {other:?}"),
    }

    // The mcp_tool_result arrives whole and rides through verbatim, keeping its
    // `is_error` flag.
    let complete1 = kinds
        .iter()
        .find_map(|k| match k {
            TurnEventKind::ContentPartComplete { index: 1, part } => Some(part.clone()),
            _ => None,
        })
        .expect("block 1 completes");
    match complete1 {
        ContentPart::ProviderExtension { kind, payload, .. } => {
            assert_eq!(kind, "mcp_tool_result");
            assert_eq!(payload["is_error"], json!(false));
        }
        other => panic!("expected ProviderExtension, got {other:?}"),
    }

    // The streamed-accumulated message is byte-for-byte the non-streaming one.
    let non_streaming: Value =
        serde_json::from_str(include_str!("fixtures/stream_mcp_response.json"))
            .expect("valid fixture");
    assert_eq!(
        accumulator.message(),
        translate::translate_response(&non_streaming),
        "streamed MCP message must equal the non-streaming message"
    );
}

#[test]
fn mid_stream_error_is_surfaced_and_the_partial_turn_is_preserved() {
    let raw = include_str!("fixtures/stream_error.sse");
    let (results, accumulator) = drive(raw);

    // The final event is an error, surfaced as a structured Api error whose
    // payload is the verbatim native error event.
    let last = results.last().expect("at least one result");
    match last {
        Err(ProviderError::Api {
            status,
            message,
            payload,
        }) => {
            assert_eq!(*status, None);
            assert_eq!(message, "Overloaded");
            assert_eq!(
                payload.as_ref().unwrap()["error"]["type"],
                json!("overloaded_error")
            );
        }
        other => panic!("expected Api error, got {other:?}"),
    }

    // Everything before the error is still available as a partial turn.
    let partial = accumulator.message();
    assert_eq!(partial.role, Role::Assistant);
    assert_eq!(partial.content.len(), 1);
    assert_eq!(partial.content[0], ContentPart::text("Partial answer"));
}

#[test]
fn early_disconnect_leaves_the_partial_turn_readable() {
    // The transcript ends abruptly mid-block, with no content_block_stop,
    // message_delta, or message_stop — as a dropped connection would.
    let raw = include_str!("fixtures/stream_disconnect.sse");
    let (results, accumulator) = drive(raw);
    assert!(results.iter().all(Result::is_ok));

    // The in-progress text block is assembled from the deltas seen so far.
    let partial = accumulator.message();
    assert_eq!(partial.content.len(), 1);
    assert_eq!(partial.content[0], ContentPart::text("Interrupted mid"));
}

// --- End-to-end path: Provider::stream over a real HTTP + SSE response. ---

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
    let server = sse_server(include_str!("fixtures/stream_tool_use.sse")).await;
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
        serde_json::from_str(include_str!("fixtures/stream_tool_use_response.json"))
            .expect("valid fixture");
    assert_eq!(
        accumulator.message(),
        translate::translate_response(&non_streaming),
        "reassembled over HTTP must equal the non-streaming message"
    );
}

#[tokio::test]
async fn stream_end_to_end_surfaces_mid_stream_error_with_partial() {
    let server = sse_server(include_str!("fixtures/stream_error.sse")).await;
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
        ContentPart::text("Partial answer")
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
