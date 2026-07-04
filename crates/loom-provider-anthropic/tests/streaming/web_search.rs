//! Web-search server-tool streaming: the provider-executed call and result
//! blocks, and streamed/non-streamed message equivalence.

use loom_core::ContentPart;
use loom_provider::TurnEventKind;
use loom_provider_anthropic::translate;
use serde_json::{json, Value};

use super::support::{drive, kinds};

#[test]
fn web_search_server_tool_streams_and_accumulates_the_provider_executed_turn() {
    let raw = include_str!("../fixtures/stream_web_search.sse");
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
    let raw = include_str!("../fixtures/stream_web_search.sse");
    let (_, accumulator) = drive(raw);

    let non_streaming: Value =
        serde_json::from_str(include_str!("../fixtures/stream_web_search_response.json"))
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
