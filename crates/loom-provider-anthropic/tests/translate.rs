//! Fixture-based tests for Anthropic request and response translation.

use loom_core::{
    CacheHint, CacheTtl, ContentPart, Conversation, ConversationOptions, Message, ProviderBinding,
    Role, ToolDefinition,
};
use loom_provider::StopReason;
use loom_provider_anthropic::translate;
use serde_json::{json, Value};
use uuid::Uuid;

/// Counts every `cache_control` marker anywhere in a native request body.
fn count_cache_control(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            usize::from(map.contains_key("cache_control"))
                + map.values().map(count_cache_control).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(count_cache_control).sum(),
        _ => 0,
    }
}

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
                cache: None,
            },
            ContentPart::text("Let me check."),
            ContentPart::ToolUse {
                id: "toolu_1".to_owned(),
                name: "get_weather".to_owned(),
                input: json!({ "location": "Wellington, NZ" }),
                cache: None,
            },
        ],
    ));

    conversation.messages.push(Message::new(
        Role::User,
        vec![ContentPart::ToolResult {
            tool_use_id: "toolu_1".to_owned(),
            content: json!([{ "type": "text", "text": "18C and clear" }]),
            is_error: Some(false),
            cache: None,
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
        cache: None,
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
fn explicit_cache_hints_place_native_cache_control() {
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.system = Some("You are Loom.".to_owned());
    conversation.system_cache = Some(CacheHint::ephemeral());
    conversation.messages.push(Message::new(
        Role::User,
        vec![ContentPart::Text {
            text: "large shared preamble".to_owned(),
            citations: None,
            cache: Some(CacheHint::with_ttl(CacheTtl::OneHour)),
        }],
    ));

    let mut options = ConversationOptions::new();
    options.tools.push(ToolDefinition {
        name: "get_weather".to_owned(),
        description: None,
        input_schema: json!({ "type": "object" }),
        cache: Some(CacheHint::with_ttl(CacheTtl::FiveMinutes)),
    });

    let request = translate::translate_request(&conversation, &options);

    // System is emitted as a cache-controlled text block (not a bare string).
    let system = request["system"].as_array().expect("system as blocks");
    assert_eq!(system[0]["type"], json!("text"));
    assert_eq!(system[0]["text"], json!("You are Loom."));
    assert_eq!(system[0]["cache_control"], json!({ "type": "ephemeral" }));

    // The tool carries a 5m cache_control marker.
    assert_eq!(
        request["tools"][0]["cache_control"],
        json!({ "type": "ephemeral", "ttl": "5m" })
    );

    // The content block carries a 1h cache_control marker.
    assert_eq!(
        request["messages"][0]["content"][0]["cache_control"],
        json!({ "type": "ephemeral", "ttl": "1h" })
    );

    // Three explicit breakpoints, all from the caller's hints.
    assert_eq!(count_cache_control(&request), 3);
    assert!(translate::requests_caching(&conversation, &options));
}

/// Auto-cache places valid, deterministic breakpoints (system head + trailing
/// message) on a long persisted conversation, staying within Anthropic's limit.
#[test]
fn auto_cache_places_valid_breakpoints_within_limit() {
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.system = Some("You are Loom.".to_owned());
    // A long history — auto-cache must place a fixed number of breakpoints
    // regardless of length.
    for i in 0..20 {
        conversation
            .messages
            .push(Message::user(format!("question {i}")));
        conversation
            .messages
            .push(Message::assistant(format!("answer {i}")));
    }

    let mut options = ConversationOptions::new();
    options.auto_cache = true;
    // Offer a tool so the head breakpoint has tools to cache alongside system.
    options.tools.push(ToolDefinition {
        name: "search".to_owned(),
        description: None,
        input_schema: json!({ "type": "object" }),
        cache: None,
    });

    let request = translate::translate_request(&conversation, &options);

    // Exactly two auto breakpoints, both valid (≤ 4).
    let count = count_cache_control(&request);
    assert_eq!(count, 2, "auto-cache should place exactly two breakpoints");
    assert!(count <= 4, "must respect Anthropic's 4-breakpoint maximum");

    // Head breakpoint on the system block (which caches the tools rendered
    // before it).
    let system = request["system"].as_array().expect("system as blocks");
    assert_eq!(system[0]["cache_control"], json!({ "type": "ephemeral" }));
    assert!(request["tools"][0].get("cache_control").is_none());

    // Trailing breakpoint on the last content block of the last message.
    let messages = request["messages"].as_array().expect("messages");
    let last = messages.last().expect("a last message");
    let last_block = last["content"].as_array().and_then(|c| c.last()).unwrap();
    assert_eq!(last_block["cache_control"], json!({ "type": "ephemeral" }));
}

/// When explicit hints already fill the breakpoint budget, auto-cache must not
/// push the request past Anthropic's four-breakpoint maximum.
#[test]
fn auto_cache_respects_the_four_breakpoint_limit() {
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.system = Some("sys".to_owned());
    // Four user turns, each already carrying an explicit cache hint → four
    // explicit breakpoints, exactly meeting the cap before auto-cache runs.
    for i in 0..4 {
        conversation.messages.push(Message::new(
            Role::User,
            vec![ContentPart::Text {
                text: format!("turn {i}"),
                citations: None,
                cache: Some(CacheHint::ephemeral()),
            }],
        ));
    }

    let mut options = ConversationOptions::new();
    options.auto_cache = true;

    let request = translate::translate_request(&conversation, &options);
    // Auto-cache must add nothing: the four explicit hints already fill the
    // budget, so the total stays at Anthropic's maximum of four.
    assert_eq!(
        count_cache_control(&request),
        4,
        "auto-cache must not exceed the 4-breakpoint maximum"
    );
}

/// A native `cache_control` marker echoed on a response block is read back onto
/// the domain [`CacheHint`] and re-emitted on the next request.
#[test]
fn cache_control_round_trips_through_response_block() {
    let native = json!({
        "type": "text",
        "text": "cached prefix",
        "cache_control": { "type": "ephemeral", "ttl": "1h" }
    });
    let part = translate::block_to_part(&native);
    match &part {
        ContentPart::Text { cache, .. } => {
            assert_eq!(*cache, Some(CacheHint::with_ttl(CacheTtl::OneHour)));
        }
        other => panic!("expected Text, got {other:?}"),
    }

    // Re-emit through a request and confirm the marker survives byte-for-byte.
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation
        .messages
        .push(Message::new(Role::Assistant, vec![part]));
    let request = translate::translate_request(&conversation, &ConversationOptions::new());
    assert_eq!(request["messages"][0]["content"][0], native);
}

/// Soft-ignore negotiation strips cache markers from a built request body while
/// leaving everything else intact.
#[test]
fn strip_cache_control_removes_every_marker() {
    let mut body = json!({
        "system": [{ "type": "text", "text": "s", "cache_control": { "type": "ephemeral" } }],
        "tools": [{ "name": "t", "cache_control": { "type": "ephemeral", "ttl": "1h" } }],
        "messages": [{
            "role": "user",
            "content": [{ "type": "text", "text": "hi", "cache_control": { "type": "ephemeral" } }]
        }]
    });
    translate::strip_cache_control(&mut body);
    assert_eq!(count_cache_control(&body), 0);
    // Non-cache fields are untouched.
    assert_eq!(body["messages"][0]["content"][0]["text"], json!("hi"));
    assert_eq!(body["tools"][0]["name"], json!("t"));
}

/// A cached fixture response splits `cache_creation_input_tokens` /
/// `cache_read_input_tokens` into the domain [`Usage`] cache fields — the
/// figures a usage rollup then sums and prices (cache-write ~1.25×, cache-read
/// ~0.10×; see the store's pricing and rollup tests).
#[test]
fn cached_response_usage_splits_cache_tokens() {
    let native = fixture();
    let usage = translate::translate_usage(&native["usage"]);
    assert_eq!(usage.cache_write_tokens, Some(64));
    assert_eq!(usage.cache_read_tokens, Some(512));
    assert_eq!(usage.input_tokens, Some(1024));
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
