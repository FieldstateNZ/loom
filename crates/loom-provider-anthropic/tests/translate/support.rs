//! Shared fixtures and helpers for the translation test modules.

use loom_core::{Conversation, Message, ProviderBinding};
use serde_json::Value;
use uuid::Uuid;

/// Counts every `cache_control` marker anywhere in a native request body.
pub(crate) fn count_cache_control(value: &Value) -> usize {
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
pub(crate) fn multi_turn() -> Conversation {
    use loom_core::{ContentPart, Role};
    use serde_json::json;

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

/// The recorded non-streaming Messages response fixture used by the response
/// translation and cache round-trip tests.
pub(crate) fn fixture() -> Value {
    serde_json::from_str(include_str!("../fixtures/messages_response.json")).expect("valid fixture")
}

/// A conversation binding used by the server-tool and MCP request tests.
pub(crate) fn bound() -> Conversation {
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.messages.push(Message::user("search the web"));
    conversation
}
