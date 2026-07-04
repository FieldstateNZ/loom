//! Response translation: mapping a native Messages response back to Loom's
//! [`loom_core::Message`], including `stop_reason` mapping and the lossless
//! request round-trip.

use loom_core::{ContentPart, Conversation, ConversationOptions, ProviderBinding, Role};
use loom_provider::StopReason;
use loom_provider_anthropic::translate;
use serde_json::json;
use uuid::Uuid;

use super::support::fixture;

#[test]
fn response_translation_maps_all_block_types() {
    let native = fixture();
    let message = translate::translate_response(&native);

    assert_eq!(message.role, Role::Assistant);

    let parts = &message.content;
    assert!(matches!(parts[0], ContentPart::Thinking { .. }));
    assert!(matches!(
        parts[1],
        ContentPart::Text {
            citations: None,
            ..
        }
    ));
    assert!(matches!(parts[2], ContentPart::ServerToolUse { .. }));
    assert!(matches!(parts[3], ContentPart::ServerToolResult { .. }));

    // Cited summary text preserves the provider citation shape verbatim.
    match &parts[4] {
        ContentPart::Text {
            citations: Some(citations),
            ..
        } => {
            assert_eq!(citations.len(), 1);
            assert_eq!(citations[0].0["type"], json!("web_search_result_location"));
        }
        other => panic!("expected cited Text, got {other:?}"),
    }

    assert!(matches!(parts[5], ContentPart::ToolUse { .. }));
}

#[test]
fn unknown_block_becomes_provider_extension() {
    let native = fixture();
    let message = translate::translate_response(&native);

    match &message.content[6] {
        ContentPart::ProviderExtension {
            provider,
            kind,
            payload,
        } => {
            assert_eq!(provider, "anthropic");
            assert_eq!(kind, "mcp_tool_use");
            assert_eq!(payload["server_name"], json!("github"));
        }
        other => panic!("expected ProviderExtension, got {other:?}"),
    }
}

#[test]
fn response_usage_maps_cache_and_server_tool_counters() {
    let native = fixture();
    let usage = translate::translate_usage(&native["usage"]);

    assert_eq!(usage.input_tokens, Some(1024));
    assert_eq!(usage.output_tokens, Some(128));
    assert_eq!(usage.cache_read_tokens, Some(512));
    assert_eq!(usage.cache_write_tokens, Some(64));
    assert_eq!(usage.server_tool_use.get("web_search_requests"), Some(&1));
    // The raw usage payload is preserved verbatim.
    assert_eq!(usage.raw.as_ref(), Some(&native["usage"]));
}

#[test]
fn response_preserves_raw_and_maps_stop_reason() {
    let native = fixture();
    let message = translate::translate_response(&native);

    // The whole verbatim native response is preserved for audit and replay.
    assert_eq!(message.raw.as_ref(), Some(&native));
    assert_eq!(translate::stop_reason(&native), Some(StopReason::ToolUse));
}

#[test]
fn stop_reason_maps_every_known_value() {
    for (native, expected) in [
        ("end_turn", StopReason::EndTurn),
        ("max_tokens", StopReason::MaxTokens),
        ("stop_sequence", StopReason::StopSequence),
        ("tool_use", StopReason::ToolUse),
        ("pause_turn", StopReason::PauseTurn),
        ("refusal", StopReason::Refusal),
    ] {
        assert_eq!(
            translate::stop_reason(&json!({ "stop_reason": native })),
            Some(expected)
        );
    }
    assert_eq!(
        translate::stop_reason(&json!({ "stop_reason": "novel" })),
        Some(StopReason::Other("novel".to_owned()))
    );
    assert_eq!(translate::stop_reason(&json!({})), None);
}

#[test]
fn response_content_round_trips_losslessly_back_to_native_blocks() {
    let native = fixture();
    let message = translate::translate_response(&native);

    // Rebuild a request from a conversation carrying the response's parts and
    // assert the assistant content array is semantically identical to the
    // fixture — proving no block was dropped or reshaped.
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.messages.push(message);

    let request = translate::translate_request(&conversation, &ConversationOptions::new());
    let rebuilt = &request["messages"][0]["content"];
    assert_eq!(
        rebuilt, &native["content"],
        "content did not round-trip losslessly"
    );
}
