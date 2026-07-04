//! Integration tests for conversation/message persistence and tenant
//! isolation, against a real database.

mod common;

use chrono::SubsecRound;
use serde_json::json;
use uuid::Uuid;

use loom_core::{
    CacheHint, Citation, ContentPart, Conversation, MediaSource, Message, ProviderBinding, Role,
    Usage,
};
use loom_store::{ConversationStore, NewTenant, TenantStore};

/// Builds a conversation exercising every content-part shape we care about,
/// including a `ProviderExtension` and a message carrying `Usage`.
fn rich_conversation(tenant_id: Uuid) -> Conversation {
    let mut conversation = Conversation::new(
        tenant_id,
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.system = Some("You are Loom.".to_owned());
    conversation.metadata = json!({ "trace_id": "abc-123", "tags": ["a", "b"] });

    conversation.messages.push(Message::new(
        Role::User,
        vec![
            ContentPart::text("Describe this image."),
            ContentPart::Image {
                source: MediaSource::Base64 {
                    media_type: "image/png".to_owned(),
                    data: "AAAA".to_owned(),
                },
                // A per-block cache hint persists through the messages JSONB
                // column and must survive the round-trip.
                cache: Some(CacheHint::ephemeral()),
            },
            ContentPart::Document {
                source: MediaSource::Url {
                    url: "https://example.com/doc.pdf".to_owned(),
                },
                cache: None,
            },
        ],
    ));

    let mut usage = Usage::new();
    usage.input_tokens = Some(42);
    usage.output_tokens = Some(7);
    usage.cache_read_tokens = Some(0);
    usage
        .server_tool_use
        .insert("web_search_requests".to_owned(), 3);
    usage.raw = Some(json!({ "native": true }));

    let assistant = Message {
        role: Role::Assistant,
        content: vec![
            ContentPart::Thinking {
                thinking: "Let me reason.".to_owned(),
                signature: Some("sig-xyz".to_owned()),
                cache: None,
            },
            ContentPart::Text {
                text: "It is a red square.".to_owned(),
                citations: Some(vec![Citation(json!({ "page": 1 }))]),
                cache: None,
            },
            ContentPart::ToolUse {
                id: "tool-1".to_owned(),
                name: "get_weather".to_owned(),
                input: json!({ "city": "Wellington" }),
                cache: None,
            },
            ContentPart::ServerToolUse {
                id: "srv-1".to_owned(),
                name: "web_search".to_owned(),
                input: json!({ "q": "loom" }),
            },
            ContentPart::ServerToolResult {
                tool_use_id: "srv-1".to_owned(),
                content: json!({ "results": [] }),
            },
            ContentPart::RedactedThinking {
                data: "opaque".to_owned(),
            },
            ContentPart::ProviderExtension {
                provider: "anthropic".to_owned(),
                kind: "mcp_tool_use".to_owned(),
                payload: json!({ "server": "fs", "args": { "nested": [1, 2, 3] } }),
            },
        ],
        usage: Some(usage),
        // Exercises lossless persistence of the verbatim provider payload
        // (messages.raw_provider_payload): the round-trip must preserve it.
        raw: Some(json!({
            "id": "msg_native_01",
            "model": "claude-opus-4-8",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "It is a red square." }]
        })),
    };
    conversation.messages.push(assistant);

    conversation.messages.push(Message::new(
        Role::Provider,
        vec![ContentPart::ToolResult {
            tool_use_id: "tool-1".to_owned(),
            content: json!({ "temp_c": 12 }),
            is_error: Some(false),
            cache: None,
        }],
    ));

    // PostgreSQL `timestamptz` stores microsecond precision; truncate so the
    // round-trip comparison is exact.
    conversation.created_at = conversation.created_at.trunc_subsecs(6);
    conversation.updated_at = conversation.updated_at.trunc_subsecs(6);
    conversation
}

#[tokio::test]
async fn conversation_history_round_trips_losslessly() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("conv", "Conv Tenant"))
        .await
        .unwrap();

    let conversation = rich_conversation(tenant.id);
    store.create_conversation(&conversation).await.unwrap();

    let reloaded = store
        .get_conversation(tenant.id, conversation.id)
        .await
        .unwrap()
        .expect("conversation exists");

    assert_eq!(
        reloaded, conversation,
        "conversation must round-trip through JSONB byte-for-byte at the domain level"
    );
}

#[tokio::test]
async fn append_and_paginate_messages() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("append", "Append Tenant"))
        .await
        .unwrap();

    let mut conversation = Conversation::new(
        tenant.id,
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.created_at = conversation.created_at.trunc_subsecs(6);
    conversation.updated_at = conversation.updated_at.trunc_subsecs(6);
    conversation.messages.push(Message::user("first"));
    store.create_conversation(&conversation).await.unwrap();

    let seq1 = store
        .append_message(tenant.id, conversation.id, &Message::assistant("second"))
        .await
        .unwrap();
    let seq2 = store
        .append_message(tenant.id, conversation.id, &Message::user("third"))
        .await
        .unwrap();
    assert_eq!(seq1, Some(1));
    assert_eq!(seq2, Some(2));

    let all = store
        .list_messages(tenant.id, conversation.id, 100, 0)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0], Message::user("first"));
    assert_eq!(all[2], Message::user("third"));

    let page = store
        .list_messages(tenant.id, conversation.id, 1, 1)
        .await
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0], Message::assistant("second"));
}

#[tokio::test]
async fn tenant_isolation_blocks_cross_tenant_reads() {
    let (_pg, store) = common::setup().await;
    let tenant_a = store
        .create_tenant(NewTenant::new("tenant-a", "Tenant A"))
        .await
        .unwrap();
    let tenant_b = store
        .create_tenant(NewTenant::new("tenant-b", "Tenant B"))
        .await
        .unwrap();

    let mut conversation = Conversation::new(
        tenant_a.id,
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.created_at = conversation.created_at.trunc_subsecs(6);
    conversation.updated_at = conversation.updated_at.trunc_subsecs(6);
    conversation.messages.push(Message::user("secret"));
    store.create_conversation(&conversation).await.unwrap();

    // Tenant A can read its own conversation.
    assert!(store
        .get_conversation(tenant_a.id, conversation.id)
        .await
        .unwrap()
        .is_some());

    // Tenant B must not see tenant A's conversation.
    assert!(
        store
            .get_conversation(tenant_b.id, conversation.id)
            .await
            .unwrap()
            .is_none(),
        "cross-tenant read must return nothing"
    );

    // Tenant A can list its own messages.
    let own_messages = store
        .list_messages(tenant_a.id, conversation.id, 100, 0)
        .await
        .unwrap();
    assert_eq!(own_messages.len(), 1);
    assert_eq!(own_messages[0], Message::user("secret"));

    // Tenant B must not read tenant A's messages, even with the id.
    let leaked = store
        .list_messages(tenant_b.id, conversation.id, 100, 0)
        .await
        .unwrap();
    assert!(
        leaked.is_empty(),
        "cross-tenant list_messages must return nothing"
    );

    // Tenant B must not be able to append to tenant A's conversation.
    let appended = store
        .append_message(tenant_b.id, conversation.id, &Message::user("intruder"))
        .await
        .unwrap();
    assert!(appended.is_none(), "cross-tenant append_message must no-op");
    // And the history must be unchanged for the owner.
    let after = store
        .list_messages(tenant_a.id, conversation.id, 100, 0)
        .await
        .unwrap();
    assert_eq!(
        after.len(),
        1,
        "cross-tenant append must not mutate history"
    );

    // Nor delete it.
    assert!(!store
        .delete_conversation(tenant_b.id, conversation.id)
        .await
        .unwrap());
    // Owner can delete it.
    assert!(store
        .delete_conversation(tenant_a.id, conversation.id)
        .await
        .unwrap());
    assert!(store
        .get_conversation(tenant_a.id, conversation.id)
        .await
        .unwrap()
        .is_none());
}
