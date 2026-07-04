//! Serde round-trip test for [`Conversation`].

mod common;

use chrono::{TimeZone, Utc};
use common::assert_json_roundtrip;
use loom_core::{CacheHint, ContentPart, Conversation, Message, ProviderBinding, Role};
use serde_json::json;
use uuid::Uuid;

#[test]
fn conversation_roundtrips() {
    let conversation = Conversation {
        id: Uuid::from_u128(1),
        tenant_id: Uuid::from_u128(2),
        binding: ProviderBinding::new("anthropic", "claude-opus-4-8"),
        system: Some("You are helpful.".into()),
        system_cache: Some(CacheHint::ephemeral()),
        messages: vec![
            Message::user("hi"),
            Message::new(
                Role::Assistant,
                vec![ContentPart::ServerToolResult {
                    tool_use_id: "srvtoolu_1".into(),
                    content: json!([{ "type": "web_search_result" }]),
                }],
            ),
        ],
        metadata: json!({ "trace_id": "abc" }),
        created_at: Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2026, 7, 3, 12, 5, 0).unwrap(),
    };
    assert_json_roundtrip(&conversation);
}
