//! Request translation: mapping a [`Conversation`] and its
//! [`ConversationOptions`] to the native request body.

use loom_core::ConversationOptions;
use loom_provider_anthropic::translate;
use serde_json::json;

use super::support::multi_turn;

#[test]
fn request_translation_is_correct_for_multi_turn_with_tools_and_thinking() {
    let conversation = multi_turn();

    let mut options = ConversationOptions::new();
    options.max_tokens = Some(2048);
    options.temperature = Some(0.5);
    options.stop_sequences = vec!["STOP".to_owned()];
    options.tools.push(loom_core::ToolDefinition {
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
