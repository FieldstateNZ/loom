//! Prompt caching: explicit [`loom_core::CacheHint`] placement, auto-cache
//! breakpoints, cache round-tripping, and stripping.

use loom_core::{
    CacheHint, CacheTtl, ContentPart, Conversation, ConversationOptions, Message, ProviderBinding,
    Role, ToolDefinition,
};
use loom_provider_anthropic::translate;
use serde_json::json;
use uuid::Uuid;

use super::support::{count_cache_control, fixture};

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
