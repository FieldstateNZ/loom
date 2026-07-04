//! Fixture-based tests for Anthropic request and response translation.

use loom_core::{
    ContentPart, Conversation, ConversationOptions, Message, ProviderBinding, Role, ToolDefinition,
};
use loom_provider::StopReason;
use loom_provider_anthropic::translate;
use serde_json::{json, Value};
use uuid::Uuid;

/// A multi-turn conversation exercising a system prompt, a client `tool_use`
/// and its `tool_result`, and a signed `thinking` block — the shapes that must
/// round-trip for multi-turn correctness.
fn multi_turn() -> Conversation {
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.system = Some("You are Loom.".to_owned());

    conversation
        .messages
        .push(Message::user("What is the weather in Wellington?"));

    conversation.messages.push(Message::new(
        Role::Assistant,
        vec![
            ContentPart::Thinking {
                thinking: "I should call the weather tool.".to_owned(),
                signature: Some("sig-abc".to_owned()),
            },
            ContentPart::text("Let me check."),
            ContentPart::ToolUse {
                id: "toolu_1".to_owned(),
                name: "get_weather".to_owned(),
                input: json!({ "location": "Wellington, NZ" }),
            },
        ],
    ));

    conversation.messages.push(Message::new(
        Role::User,
        vec![ContentPart::ToolResult {
            tool_use_id: "toolu_1".to_owned(),
            content: json!([{ "type": "text", "text": "18C and clear" }]),
            is_error: Some(false),
        }],
    ));

    conversation
}

#[test]
fn request_translation_is_correct_for_multi_turn_with_tools_and_thinking() {
    let conversation = multi_turn();

    let mut options = ConversationOptions::new();
    options.max_tokens = Some(2048);
    options.temperature = Some(0.5);
    options.stop_sequences = vec!["STOP".to_owned()];
    options.tools.push(ToolDefinition {
        name: "get_weather".to_owned(),
        description: Some("Look up the weather".to_owned()),
        input_schema: json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"]
        }),
    });
    // The native options bag is merged transparently — tool_choice and top_p
    // ride through without a Loom release.
    options.provider_options.insert(
        "anthropic".to_owned(),
        json!({ "tool_choice": { "type": "auto" }, "top_p": 0.9 }),
    );

    let request = translate::translate_request(&conversation, &options);

    assert_eq!(request["model"], json!("claude-opus-4-8"));
    assert_eq!(request["max_tokens"], json!(2048));
    assert_eq!(request["system"], json!("You are Loom."));
    assert_eq!(request["temperature"], json!(0.5));
    assert_eq!(request["stop_sequences"], json!(["STOP"]));

    // Merged native options.
    assert_eq!(request["tool_choice"], json!({ "type": "auto" }));
    assert_eq!(request["top_p"], json!(0.9));

    // Tools.
    let tools = request["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], json!("get_weather"));
    assert_eq!(tools[0]["description"], json!("Look up the weather"));
    assert_eq!(tools[0]["input_schema"]["type"], json!("object"));

    // Messages: three turns with faithful native blocks.
    let messages = request["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 3);

    assert_eq!(messages[0]["role"], json!("user"));
    assert_eq!(messages[0]["content"][0]["type"], json!("text"));
    assert_eq!(
        messages[0]["content"][0]["text"],
        json!("What is the weather in Wellington?")
    );

    // Thinking block round-trips with its signature intact.
    assert_eq!(messages[1]["role"], json!("assistant"));
    let assistant_blocks = messages[1]["content"]
        .as_array()
        .expect("assistant content");
    assert_eq!(assistant_blocks[0]["type"], json!("thinking"));
    assert_eq!(
        assistant_blocks[0]["thinking"],
        json!("I should call the weather tool.")
    );
    assert_eq!(assistant_blocks[0]["signature"], json!("sig-abc"));
    assert_eq!(assistant_blocks[2]["type"], json!("tool_use"));
    assert_eq!(assistant_blocks[2]["id"], json!("toolu_1"));
    assert_eq!(assistant_blocks[2]["name"], json!("get_weather"));

    // The client tool_result is sent back on a user turn.
    assert_eq!(messages[2]["role"], json!("user"));
    assert_eq!(messages[2]["content"][0]["type"], json!("tool_result"));
    assert_eq!(messages[2]["content"][0]["tool_use_id"], json!("toolu_1"));
    assert_eq!(messages[2]["content"][0]["is_error"], json!(false));
}

#[test]
fn request_defaults_max_tokens_when_unset() {
    let conversation = multi_turn();
    let request = translate::translate_request(&conversation, &ConversationOptions::new());
    // Anthropic requires max_tokens; a default is supplied.
    assert!(request["max_tokens"].as_u64().unwrap() > 0);
    // With no tools offered, the field is omitted entirely.
    assert!(request.get("tools").is_none());
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/messages_response.json")).expect("valid fixture")
}

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
