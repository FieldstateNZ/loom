//! Integration tests exercising the PostgreSQL store against a real database.
//!
//! Each test spins up a throwaway PostgreSQL 16 container via `testcontainers`,
//! applies the embedded migrations, and drives the store traits.

use chrono::SubsecRound;
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

use loom_core::{
    Citation, ContentPart, Conversation, MediaSource, Message, ProviderBinding, Role, Usage,
};
use loom_store::{
    run_migrations, ConversationStore, CredentialStore, KeyBudget, KeyStore, NewProviderCredential,
    NewTenant, NewUsageEvent, NewVirtualKey, PgStore, TenantStore, UsageStore,
};

use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers::ImageExt;
use testcontainers_modules::postgres::Postgres;

/// Boots a fresh, migrated database and returns the live container (which must
/// be kept alive for the duration of the test) plus a connected store.
async fn setup() -> (ContainerAsync<Postgres>, PgStore) {
    let container = Postgres::default()
        .with_tag("16")
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("map postgres port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let store = PgStore::connect(&url).await.expect("connect to postgres");
    run_migrations(store.pool())
        .await
        .expect("migrations apply cleanly from empty database");
    (container, store)
}

#[tokio::test]
async fn migrations_apply_and_are_idempotent() {
    let (_pg, store) = setup().await;
    // Running again must be a no-op rather than an error.
    run_migrations(store.pool())
        .await
        .expect("re-running migrations is idempotent");
}

#[tokio::test]
async fn tenant_crud() {
    let (_pg, store) = setup().await;

    let created = store
        .create_tenant(NewTenant::new("acme", "Acme Inc"))
        .await
        .unwrap();
    assert_eq!(created.slug, "acme");
    assert_eq!(created.name, "Acme Inc");
    assert_eq!(created.status, "active");

    let by_id = store.get_tenant(created.id).await.unwrap().unwrap();
    assert_eq!(by_id, created);

    let by_slug = store.get_tenant_by_slug("acme").await.unwrap().unwrap();
    assert_eq!(by_slug, created);

    assert!(store.get_tenant(Uuid::new_v4()).await.unwrap().is_none());
    assert!(store.get_tenant_by_slug("nope").await.unwrap().is_none());
}

#[tokio::test]
async fn virtual_key_crud() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("keys", "Keys Tenant"))
        .await
        .unwrap();

    let key = store
        .create_key(NewVirtualKey {
            tenant_id: tenant.id,
            key_hash: "hash-abc".to_owned(),
            key_prefix: "sk-live-abc".to_owned(),
            name: "prod".to_owned(),
            scopes: vec!["chat".to_owned(), "usage:read".to_owned()],
            budget: Some(KeyBudget {
                limit_amount: Decimal::new(2500, 2),
                window: "monthly".to_owned(),
                action: "block".to_owned(),
            }),
        })
        .await
        .unwrap();
    assert_eq!(key.status, "active");
    assert_eq!(key.scopes, vec!["chat", "usage:read"]);
    assert_eq!(
        key.budget.as_ref().unwrap().limit_amount,
        Decimal::new(2500, 2)
    );
    assert!(key.last_used_at.is_none());

    let fetched = store.get_key_by_hash("hash-abc").await.unwrap().unwrap();
    assert_eq!(fetched, key);

    assert!(store.touch_key_last_used(key.id).await.unwrap());
    let touched = store.get_key_by_hash("hash-abc").await.unwrap().unwrap();
    assert!(touched.last_used_at.is_some());

    assert!(store.revoke_key(key.id).await.unwrap());
    let revoked = store.get_key_by_hash("hash-abc").await.unwrap().unwrap();
    assert_eq!(revoked.status, "revoked");

    assert!(store.get_key_by_hash("missing").await.unwrap().is_none());
    assert!(!store.revoke_key(Uuid::new_v4()).await.unwrap());
}

#[tokio::test]
async fn credential_upsert_and_global() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("creds", "Creds Tenant"))
        .await
        .unwrap();

    // Tenant-scoped credential.
    let cred = store
        .upsert_credential(NewProviderCredential {
            tenant_id: Some(tenant.id),
            provider: "anthropic".to_owned(),
            encrypted_secret: vec![1, 2, 3, 4],
            nonce: Some(vec![9, 9]),
            aad: None,
            base_url: Some("https://api.anthropic.com".to_owned()),
        })
        .await
        .unwrap();
    assert_eq!(cred.tenant_id, Some(tenant.id));
    assert_eq!(cred.encrypted_secret, vec![1, 2, 3, 4]);

    // Upsert replaces the secret for the same (tenant, provider) pair.
    let updated = store
        .upsert_credential(NewProviderCredential {
            tenant_id: Some(tenant.id),
            provider: "anthropic".to_owned(),
            encrypted_secret: vec![5, 6, 7, 8],
            nonce: Some(vec![1]),
            aad: Some(vec![2]),
            base_url: None,
        })
        .await
        .unwrap();
    assert_eq!(updated.id, cred.id, "upsert keeps the same row");
    assert_eq!(updated.encrypted_secret, vec![5, 6, 7, 8]);
    assert_eq!(updated.base_url, None);

    // Gateway-global credential (NULL tenant) coexists with the tenant one.
    let global = store
        .upsert_credential(NewProviderCredential {
            tenant_id: None,
            provider: "anthropic".to_owned(),
            encrypted_secret: vec![0],
            nonce: None,
            aad: None,
            base_url: None,
        })
        .await
        .unwrap();
    assert_eq!(global.tenant_id, None);

    let got = store
        .get_credential(Some(tenant.id), "anthropic")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, updated);

    let got_global = store
        .get_credential(None, "anthropic")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_global, global);

    let tenant_list = store.list_credentials(Some(tenant.id)).await.unwrap();
    assert_eq!(tenant_list.len(), 1);
}

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
            },
            ContentPart::Document {
                source: MediaSource::Url {
                    url: "https://example.com/doc.pdf".to_owned(),
                },
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
            },
            ContentPart::Text {
                text: "It is a red square.".to_owned(),
                citations: Some(vec![Citation(json!({ "page": 1 }))]),
            },
            ContentPart::ToolUse {
                id: "tool-1".to_owned(),
                name: "get_weather".to_owned(),
                input: json!({ "city": "Wellington" }),
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
        raw: None,
    };
    conversation.messages.push(assistant);

    conversation.messages.push(Message::new(
        Role::Provider,
        vec![ContentPart::ToolResult {
            tool_use_id: "tool-1".to_owned(),
            content: json!({ "temp_c": 12 }),
            is_error: Some(false),
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
    let (_pg, store) = setup().await;
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
    let (_pg, store) = setup().await;
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
        .append_message(conversation.id, &Message::assistant("second"))
        .await
        .unwrap();
    let seq2 = store
        .append_message(conversation.id, &Message::user("third"))
        .await
        .unwrap();
    assert_eq!(seq1, 1);
    assert_eq!(seq2, 2);

    let all = store.list_messages(conversation.id, 100, 0).await.unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0], Message::user("first"));
    assert_eq!(all[2], Message::user("third"));

    let page = store.list_messages(conversation.id, 1, 1).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0], Message::assistant("second"));
}

#[tokio::test]
async fn tenant_isolation_blocks_cross_tenant_reads() {
    let (_pg, store) = setup().await;
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

#[tokio::test]
async fn usage_events_record_and_rollup() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("usage", "Usage Tenant"))
        .await
        .unwrap();
    let other = store
        .create_tenant(NewTenant::new("usage-other", "Other"))
        .await
        .unwrap();

    let mut usage = Usage::new();
    usage.input_tokens = Some(100);
    usage.output_tokens = Some(20);
    usage.cache_read_tokens = Some(5);
    usage.cache_write_tokens = Some(3);
    usage
        .server_tool_use
        .insert("web_search_requests".to_owned(), 2);
    usage.raw = Some(json!({ "provider": "anthropic" }));

    store
        .record_event(NewUsageEvent {
            tenant_id: tenant.id,
            virtual_key_id: None,
            conversation_id: None,
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            usage: usage.clone(),
            cost: Some(Decimal::new(1234, 4)),
        })
        .await
        .unwrap();
    store
        .record_event(NewUsageEvent {
            tenant_id: tenant.id,
            virtual_key_id: None,
            conversation_id: None,
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            usage: usage.clone(),
            cost: None,
        })
        .await
        .unwrap();
    // A different tenant's event must not leak into the rollup.
    store
        .record_event(NewUsageEvent {
            tenant_id: other.id,
            virtual_key_id: None,
            conversation_id: None,
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            usage: usage.clone(),
            cost: None,
        })
        .await
        .unwrap();

    let events = store.list_events(tenant.id, 100).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].input_tokens, 100);
    assert_eq!(
        events[0].server_tool_counts,
        json!({ "web_search_requests": 2 })
    );

    let rollup = store.rollup(tenant.id).await.unwrap();
    assert_eq!(rollup.event_count, 2);
    assert_eq!(rollup.input_tokens, 200);
    assert_eq!(rollup.output_tokens, 40);
    assert_eq!(rollup.cache_read_tokens, 10);
    assert_eq!(rollup.cache_write_tokens, 6);
}
