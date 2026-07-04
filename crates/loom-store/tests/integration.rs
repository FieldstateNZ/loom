//! Integration tests exercising the PostgreSQL store against a real database.
//!
//! Each test spins up a throwaway PostgreSQL 16 container via `testcontainers`,
//! applies the embedded migrations, and drives the store traits.

use chrono::{SubsecRound, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

use loom_core::{
    CacheHint, Citation, ContentPart, Conversation, MediaSource, Message, ProviderBinding, Role,
    Usage,
};
use loom_store::{
    drain_usage_outbox, run_migrations, BatchCounts, BatchItemStatus, BatchStatus, BatchStore,
    BudgetAction, BudgetStore, BudgetWindow, ConversationStore, CredentialStore, KeyBudget,
    KeyStore, McpServerStore, NewBatchItem, NewBatchJob, NewMcpServer, NewModelPrice,
    NewProviderCredential, NewTenant, NewUsageEvent, NewVirtualKey, OutboxStore, PgStore, Pricer,
    PricingStore, RateLimit, RollupGroup, TenantStore, UsageStore,
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
                window: BudgetWindow::Monthly,
                action: BudgetAction::Block,
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

#[tokio::test]
async fn mcp_server_upsert_get_list_delete_and_tenant_isolation() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("mcp", "MCP Tenant"))
        .await
        .unwrap();
    let other = store
        .create_tenant(NewTenant::new("mcp-other", "Other Tenant"))
        .await
        .unwrap();

    let created = store
        .upsert_mcp_server(NewMcpServer {
            tenant_id: tenant.id,
            name: "github".to_owned(),
            url: "https://mcp.githubcopilot.com/mcp".to_owned(),
            encrypted_token: Some(vec![1, 2, 3, 4]),
            nonce: Some(vec![9, 9]),
            tool_configuration: Some(json!({ "enabled": true })),
        })
        .await
        .unwrap();
    assert_eq!(created.name, "github");
    assert_eq!(created.encrypted_token, Some(vec![1, 2, 3, 4]));

    // Upsert replaces the row for the same (tenant, name) pair and bumps it.
    let updated = store
        .upsert_mcp_server(NewMcpServer {
            tenant_id: tenant.id,
            name: "github".to_owned(),
            url: "https://mcp.example.com/mcp".to_owned(),
            encrypted_token: Some(vec![5, 6]),
            nonce: Some(vec![1]),
            tool_configuration: None,
        })
        .await
        .unwrap();
    assert_eq!(updated.id, created.id, "upsert keeps the same row");
    assert_eq!(updated.url, "https://mcp.example.com/mcp");
    assert_eq!(updated.encrypted_token, Some(vec![5, 6]));
    assert_eq!(updated.tool_configuration, None);

    // Fetch by name is tenant-scoped.
    let got = store
        .get_mcp_server(tenant.id, "github")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, updated);
    // Another tenant cannot see it, even by the same name.
    assert!(store
        .get_mcp_server(other.id, "github")
        .await
        .unwrap()
        .is_none());

    // A no-auth server is allowed (NULL token/nonce).
    store
        .upsert_mcp_server(NewMcpServer {
            tenant_id: tenant.id,
            name: "public".to_owned(),
            url: "https://mcp.public.example/mcp".to_owned(),
            encrypted_token: None,
            nonce: None,
            tool_configuration: None,
        })
        .await
        .unwrap();

    let list = store.list_mcp_servers(tenant.id).await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].name, "github", "ordered by name");
    assert_eq!(list[1].name, "public");
    assert!(store.list_mcp_servers(other.id).await.unwrap().is_empty());

    // Delete is tenant-scoped: another tenant's delete is a no-op.
    assert!(!store.delete_mcp_server(other.id, "github").await.unwrap());
    assert!(store.delete_mcp_server(tenant.id, "github").await.unwrap());
    assert!(store
        .get_mcp_server(tenant.id, "github")
        .await
        .unwrap()
        .is_none());
    assert!(!store.delete_mcp_server(tenant.id, "github").await.unwrap());
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
            is_batch: false,
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
            is_batch: false,
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
            is_batch: false,
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

/// Server-tool usage (a web-search request count) is priced from the seeded
/// per-request rate and shows up in a grouped rollup's cost — the end-to-end
/// "server-tool usage priced in a rollup" path for issue #12.
#[tokio::test]
async fn server_tool_usage_is_priced_into_a_rollup() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("srvtool", "Server Tool Tenant"))
        .await
        .unwrap();

    // A turn that used the web-search server tool three times, with no tokens so
    // the whole cost comes from the server-tool charge.
    let mut usage = Usage::new();
    usage
        .server_tool_use
        .insert("web_search_requests".to_owned(), 3);

    // Price it exactly as the turn runner does: the effective seeded price for
    // (anthropic, claude-opus-4-8) at the event instant.
    let at = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
    let price = store
        .get_effective_price("anthropic", "claude-opus-4-8", at)
        .await
        .unwrap()
        .expect("seeded opus price");
    let cost = Pricer::cost(&usage, &price);
    // Seeded web_search_requests price is 0.01/request → 3 * 0.01 = 0.03.
    assert_eq!(cost, Decimal::new(3, 2));

    store
        .record_event(NewUsageEvent {
            tenant_id: tenant.id,
            virtual_key_id: None,
            conversation_id: None,
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            usage: usage.clone(),
            cost: Some(cost),
            is_batch: false,
        })
        .await
        .unwrap();

    let rows = store
        .rollup_grouped(tenant.id, None, None, RollupGroup::Model)
        .await
        .unwrap();
    let opus = rows
        .iter()
        .find(|row| row.group.as_deref() == Some("claude-opus-4-8"))
        .expect("opus rollup row");
    assert_eq!(opus.event_count, 1);
    // The server-tool charge is summed into the rollup's cost.
    assert_eq!(opus.cost, Decimal::new(3, 2));
}

/// The seeded Anthropic prices load, and a newer price version supersedes the
/// old one only from its `effective_from` onward — older instants keep the old
/// price. This is the core of "price versioning affects new events only".
#[tokio::test]
async fn pricing_is_versioned_by_effective_from() {
    let (_pg, store) = setup().await;

    let early = Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap();
    let late = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();

    // The migration seeds opus at 5 / 25 effective 2026-01-01.
    let seeded = store
        .get_effective_price("anthropic", "claude-opus-4-8", early)
        .await
        .unwrap()
        .expect("seeded opus price");
    assert_eq!(seeded.input_per_mtok, Decimal::from(5));
    assert_eq!(seeded.output_per_mtok, Decimal::from(25));

    // A NEW version, effective 2026-07-01, never overwrites the old row.
    let bump_from = Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap();
    store
        .upsert_price(NewModelPrice {
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            input_per_mtok: Decimal::from(9),
            output_per_mtok: Decimal::from(45),
            cache_write_per_mtok: Decimal::new(1125, 2),
            cache_read_per_mtok: Decimal::new(90, 2),
            server_tool_prices: json!({ "web_search_requests": 0.02 }),
            batch_multiplier: rust_decimal::Decimal::ONE,
            currency: "USD".to_owned(),
            effective_from: bump_from,
        })
        .await
        .unwrap();

    // Before the bump: still the old price. After: the new one.
    let before = store
        .get_effective_price("anthropic", "claude-opus-4-8", early)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.input_per_mtok, Decimal::from(5));
    let after = store
        .get_effective_price("anthropic", "claude-opus-4-8", late)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.input_per_mtok, Decimal::from(9));

    // An unpriced model yields None (cost stays uncomputed, never an error).
    assert!(store
        .get_effective_price("anthropic", "no-such-model", late)
        .await
        .unwrap()
        .is_none());

    // The Pricer prices a cache-split usage against the seeded opus row.
    let mut usage = Usage::new();
    usage.input_tokens = Some(1_000_000);
    usage.output_tokens = Some(1_000_000);
    usage.cache_write_tokens = Some(1_000_000);
    usage.cache_read_tokens = Some(1_000_000);
    // 5 + 25 + 6.25 + 0.50 = 36.75
    assert_eq!(Pricer::cost(&usage, &seeded), Decimal::new(3675, 2));
}

/// Grouped rollups sum tokens and cost per key / model / conversation for a
/// tenant, and the gateway-wide rollup groups by tenant.
#[tokio::test]
async fn grouped_rollups_aggregate_per_dimension() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("roll", "Rollup Tenant"))
        .await
        .unwrap();
    let other = store
        .create_tenant(NewTenant::new("roll-other", "Other Tenant"))
        .await
        .unwrap();

    let key_a = store
        .create_key(NewVirtualKey {
            tenant_id: tenant.id,
            key_hash: "hash-a".to_owned(),
            key_prefix: "loom_a".to_owned(),
            name: "a".to_owned(),
            scopes: Vec::new(),
            budget: None,
        })
        .await
        .unwrap();
    let key_b = store
        .create_key(NewVirtualKey {
            tenant_id: tenant.id,
            key_hash: "hash-b".to_owned(),
            key_prefix: "loom_b".to_owned(),
            name: "b".to_owned(),
            scopes: Vec::new(),
            budget: None,
        })
        .await
        .unwrap();

    let convo = Conversation::new(
        tenant.id,
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    store.create_conversation(&convo).await.unwrap();

    let mut usage = Usage::new();
    usage.input_tokens = Some(100);
    usage.output_tokens = Some(10);

    // key_a: two events on opus, one tied to `convo`, costs 1.00 + 2.00.
    for (cost, convo_id) in [(Decimal::from(1), Some(convo.id)), (Decimal::from(2), None)] {
        store
            .record_event(NewUsageEvent {
                tenant_id: tenant.id,
                virtual_key_id: Some(key_a.id),
                conversation_id: convo_id,
                provider: "anthropic".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                usage: usage.clone(),
                cost: Some(cost),
                is_batch: false,
            })
            .await
            .unwrap();
    }
    // key_b: one event on sonnet, cost 4.00.
    store
        .record_event(NewUsageEvent {
            tenant_id: tenant.id,
            virtual_key_id: Some(key_b.id),
            conversation_id: None,
            provider: "anthropic".to_owned(),
            model: "claude-sonnet-5".to_owned(),
            usage: usage.clone(),
            cost: Some(Decimal::from(4)),
            is_batch: false,
        })
        .await
        .unwrap();
    // A different tenant's event must not leak into tenant rollups.
    store
        .record_event(NewUsageEvent {
            tenant_id: other.id,
            virtual_key_id: None,
            conversation_id: None,
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            usage: usage.clone(),
            cost: Some(Decimal::from(100)),
            is_batch: false,
        })
        .await
        .unwrap();

    // By key: two groups, key_a totals cost 3, key_b cost 4.
    let by_key = store
        .rollup_grouped(tenant.id, None, None, RollupGroup::Key)
        .await
        .unwrap();
    assert_eq!(by_key.len(), 2);
    let a = by_key
        .iter()
        .find(|r| r.group.as_deref() == Some(key_a.id.to_string().as_str()))
        .unwrap();
    assert_eq!(a.event_count, 2);
    assert_eq!(a.input_tokens, 200);
    assert_eq!(a.cost, Decimal::from(3));

    // By model: opus cost 3, sonnet cost 4.
    let by_model = store
        .rollup_grouped(tenant.id, None, None, RollupGroup::Model)
        .await
        .unwrap();
    let opus = by_model
        .iter()
        .find(|r| r.group.as_deref() == Some("claude-opus-4-8"))
        .unwrap();
    assert_eq!(opus.cost, Decimal::from(3));
    let sonnet = by_model
        .iter()
        .find(|r| r.group.as_deref() == Some("claude-sonnet-5"))
        .unwrap();
    assert_eq!(sonnet.cost, Decimal::from(4));

    // By conversation: one group tied to `convo` (cost 1) and one null group
    // (the two conversation-less events, cost 6).
    let by_convo = store
        .rollup_grouped(tenant.id, None, None, RollupGroup::Conversation)
        .await
        .unwrap();
    let tied = by_convo
        .iter()
        .find(|r| r.group.as_deref() == Some(convo.id.to_string().as_str()))
        .unwrap();
    assert_eq!(tied.event_count, 1);
    assert_eq!(tied.cost, Decimal::from(1));
    let untied = by_convo.iter().find(|r| r.group.is_none()).unwrap();
    assert_eq!(untied.cost, Decimal::from(6));

    // Gateway-wide by tenant sees both tenants.
    let by_tenant = store.rollup_by_tenant(None, None).await.unwrap();
    assert_eq!(by_tenant.len(), 2);
    let this = by_tenant
        .iter()
        .find(|r| r.group.as_deref() == Some(tenant.id.to_string().as_str()))
        .unwrap();
    assert_eq!(this.cost, Decimal::from(7));
}

/// The outbox parks a usage event and the drain path replays it into
/// `usage_events`; an event that keeps failing stays pending with a bumped
/// attempt count.
#[tokio::test]
async fn outbox_enqueue_and_drain() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("outbox", "Outbox Tenant"))
        .await
        .unwrap();

    let mut usage = Usage::new();
    usage.input_tokens = Some(42);

    // A replayable event (valid tenant FK).
    let good = NewUsageEvent {
        tenant_id: tenant.id,
        virtual_key_id: None,
        conversation_id: None,
        provider: "anthropic".to_owned(),
        model: "claude-opus-4-8".to_owned(),
        usage: usage.clone(),
        cost: Some(Decimal::from(1)),
        is_batch: false,
    };
    // An event that will fail on replay: its tenant does not exist (FK violation).
    let doomed = NewUsageEvent {
        tenant_id: Uuid::new_v4(),
        ..good.clone()
    };

    store.enqueue_outbox(&good).await.unwrap();
    store.enqueue_outbox(&doomed).await.unwrap();
    assert_eq!(store.list_pending_outbox(100).await.unwrap().len(), 2);

    let report = drain_usage_outbox(&store, 100).await.unwrap();
    assert_eq!(report.processed, 1);
    assert_eq!(report.failed, 1);

    // The good event landed in usage_events; the doomed one is still pending
    // with an attempt recorded, ready for a later retry.
    let events = store.list_events(tenant.id, 100).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].input_tokens, 42);

    let pending = store.list_pending_outbox(100).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].attempts, 1);
    assert!(pending[0].last_error.is_some());
}

/// Tenant and key budgets round-trip through the store, and `budget_spend`
/// sums recorded cost scoped to the tenant or a single key over a time window.
#[tokio::test]
async fn budgets_and_spend_scope_correctly() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("bud", "Budget Tenant"))
        .await
        .unwrap();

    // No budget on a fresh tenant.
    assert!(store.get_tenant_budget(tenant.id).await.unwrap().is_none());

    // Set, read back, then clear the tenant budget.
    let budget = KeyBudget {
        limit_amount: Decimal::from(50),
        window: BudgetWindow::Monthly,
        action: BudgetAction::Warn,
    };
    assert!(store
        .set_tenant_budget(tenant.id, Some(budget.clone()))
        .await
        .unwrap());
    assert_eq!(
        store.get_tenant_budget(tenant.id).await.unwrap(),
        Some(budget)
    );
    assert!(store.set_tenant_budget(tenant.id, None).await.unwrap());
    assert!(store.get_tenant_budget(tenant.id).await.unwrap().is_none());
    // A missing tenant reports no update.
    assert!(!store.set_tenant_budget(Uuid::new_v4(), None).await.unwrap());

    let key = store
        .create_key(NewVirtualKey {
            tenant_id: tenant.id,
            key_hash: "hash-bud".to_owned(),
            key_prefix: "loom_bud".to_owned(),
            name: "bud".to_owned(),
            scopes: Vec::new(),
            budget: None,
        })
        .await
        .unwrap();

    // Key budget + rate limit round-trip through get_key_by_hash.
    assert!(store
        .set_key_budget(
            key.id,
            Some(KeyBudget {
                limit_amount: Decimal::from(10),
                window: BudgetWindow::Daily,
                action: BudgetAction::Block,
            }),
        )
        .await
        .unwrap());
    assert!(store
        .set_key_rate_limit(
            key.id,
            Some(RateLimit {
                requests_per_min: Some(30),
                tokens_per_min: Some(90_000),
            }),
        )
        .await
        .unwrap());
    let fetched = store.get_key_by_hash("hash-bud").await.unwrap().unwrap();
    assert_eq!(fetched.budget.unwrap().limit_amount, Decimal::from(10));
    assert_eq!(fetched.rate_limit.unwrap().requests_per_min, Some(30));

    // Seed spend: the key spends 7, a second (keyless) event spends 5.
    let mut usage = Usage::new();
    usage.input_tokens = Some(1);
    store
        .record_event(NewUsageEvent {
            tenant_id: tenant.id,
            virtual_key_id: Some(key.id),
            conversation_id: None,
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            usage: usage.clone(),
            cost: Some(Decimal::from(7)),
            is_batch: false,
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
            usage,
            cost: Some(Decimal::from(5)),
            is_batch: false,
        })
        .await
        .unwrap();

    // Tenant-wide spend sees both events (12); key-scoped spend sees only 7.
    assert_eq!(
        store.budget_spend(tenant.id, None, None).await.unwrap(),
        Decimal::from(12)
    );
    assert_eq!(
        store
            .budget_spend(tenant.id, Some(key.id), None)
            .await
            .unwrap(),
        Decimal::from(7)
    );
    // A future lower bound excludes the just-recorded events.
    let future = Utc::now() + chrono::Duration::hours(1);
    assert_eq!(
        store
            .budget_spend(tenant.id, None, Some(future))
            .await
            .unwrap(),
        Decimal::ZERO
    );
}

/// The batch store round-trips a job through its lifecycle: create with items,
/// submit, persist per-item results, finalise, and read back — all tenant-scoped.
#[tokio::test]
async fn batch_job_lifecycle_and_tenant_scope() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("batch", "Batch Tenant"))
        .await
        .unwrap();
    let other = store
        .create_tenant(NewTenant::new("other-batch", "Other"))
        .await
        .unwrap();

    let job = store
        .create_batch_job(NewBatchJob {
            tenant_id: tenant.id,
            virtual_key_id: None,
            provider: "anthropic".to_owned(),
            items: vec![
                NewBatchItem {
                    custom_id: "a".to_owned(),
                    model: "claude-opus-4-8".to_owned(),
                    request: json!({ "provider": "anthropic", "model": "claude-opus-4-8" }),
                },
                NewBatchItem {
                    custom_id: "b".to_owned(),
                    model: "claude-opus-4-8".to_owned(),
                    request: json!({ "provider": "anthropic", "model": "claude-opus-4-8" }),
                },
            ],
        })
        .await
        .unwrap();
    assert_eq!(job.status, BatchStatus::Created);
    assert_eq!(job.total_items, 2);

    // Items land pending, in submission order.
    let items = store.get_batch_items(job.id).await.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].custom_id, "a");
    assert_eq!(items[0].status, BatchItemStatus::Pending);

    // The worker sees it as active until it ends.
    let active = store.list_active_batch_jobs(10).await.unwrap();
    assert_eq!(active.len(), 1);

    // Claim (created → submitting) then submit (submitting → in_progress). The
    // claim is exactly-once: a second attempt on the same job wins no row.
    assert!(store
        .claim_batch_for_submission(tenant.id, job.id)
        .await
        .unwrap());
    assert!(
        !store
            .claim_batch_for_submission(tenant.id, job.id)
            .await
            .unwrap(),
        "a job already claimed for submission cannot be claimed again"
    );
    store
        .mark_batch_submitted(
            tenant.id,
            job.id,
            "msgbatch_1",
            BatchCounts {
                processing: 2,
                ..BatchCounts::default()
            },
        )
        .await
        .unwrap();
    let submitted = store
        .get_batch_job(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(submitted.status, BatchStatus::InProgress);
    assert_eq!(submitted.provider_batch_id.as_deref(), Some("msgbatch_1"));
    assert_eq!(submitted.counts.processing, 2);

    // Persist results and finalise.
    for cid in ["a", "b"] {
        store
            .save_batch_item_result(
                tenant.id,
                job.id,
                cid,
                BatchItemStatus::Succeeded,
                &json!({ "type": "succeeded" }),
            )
            .await
            .unwrap();
    }
    store
        .update_batch_progress(
            tenant.id,
            job.id,
            BatchStatus::Ended,
            BatchCounts {
                succeeded: 2,
                ..BatchCounts::default()
            },
            Some("https://example/results"),
            Some(Utc::now()),
        )
        .await
        .unwrap();

    let ended = store
        .get_batch_job(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ended.status, BatchStatus::Ended);
    assert_eq!(ended.counts.succeeded, 2);
    assert!(ended.ended_at.is_some());
    assert!(store.list_active_batch_jobs(10).await.unwrap().is_empty());

    // Tenant-scoped reads: a foreign tenant sees nothing.
    let scoped = store.list_batch_items(tenant.id, job.id).await.unwrap();
    assert_eq!(scoped.len(), 2);
    assert!(scoped.iter().all(|i| i.result.is_some()));
    assert!(store
        .get_batch_job(other.id, job.id)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .list_batch_items(other.id, job.id)
        .await
        .unwrap()
        .is_empty());
}

/// Cancelling a job that was never submitted finalises it immediately, marking
/// every item canceled without any provider round-trip.
#[tokio::test]
async fn batch_cancel_before_submission_finalises_locally() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("precancel", "Pre-cancel"))
        .await
        .unwrap();
    let job = store
        .create_batch_job(NewBatchJob {
            tenant_id: tenant.id,
            virtual_key_id: None,
            provider: "anthropic".to_owned(),
            items: vec![NewBatchItem {
                custom_id: "only".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                request: json!({ "provider": "anthropic", "model": "claude-opus-4-8" }),
            }],
        })
        .await
        .unwrap();

    let canceled = store
        .request_batch_cancel(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(canceled.status, BatchStatus::Ended);
    assert_eq!(canceled.counts.canceled, 1);
    assert!(canceled.ended_at.is_some());

    let items = store.get_batch_items(job.id).await.unwrap();
    assert_eq!(items[0].status, BatchItemStatus::Canceled);

    // A foreign tenant cannot cancel it.
    let intruder = store
        .create_tenant(NewTenant::new("intruder-b", "Intruder"))
        .await
        .unwrap();
    assert!(store
        .request_batch_cancel(intruder.id, job.id)
        .await
        .unwrap()
        .is_none());
}

/// A cancellation that arrives while a job is being submitted must not be
/// clobbered by the submit completing: the job stays `canceling` (never goes
/// live) and, once the provider batch id is recorded, the worker can relay the
/// cancel. This is the store-level guarantee behind finding #2.
#[tokio::test]
async fn cancel_during_submitting_is_not_resurrected() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("cancel-race", "Cancel Race"))
        .await
        .unwrap();
    let job = store
        .create_batch_job(NewBatchJob {
            tenant_id: tenant.id,
            virtual_key_id: None,
            provider: "anthropic".to_owned(),
            items: vec![NewBatchItem {
                custom_id: "only".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                request: json!({ "provider": "anthropic", "model": "claude-opus-4-8" }),
            }],
        })
        .await
        .unwrap();

    // Worker claims the job for submission (created → submitting).
    assert!(store
        .claim_batch_for_submission(tenant.id, job.id)
        .await
        .unwrap());

    // A cancel arrives *during* submission: it must move the job to `canceling`,
    // NOT finalise it locally (which a concurrent submit could clobber).
    let canceling = store
        .request_batch_cancel(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(canceling.status, BatchStatus::Canceling);

    // The submit now completes. `mark_batch_submitted` must keep the job
    // `canceling` (the guarded CASE) while still recording the provider batch id
    // so the cancellation can reach the live provider batch.
    store
        .mark_batch_submitted(
            tenant.id,
            job.id,
            "msgbatch_race",
            BatchCounts {
                processing: 1,
                ..BatchCounts::default()
            },
        )
        .await
        .unwrap();
    let after = store
        .get_batch_job(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.status,
        BatchStatus::Canceling,
        "a cancel during submitting must not be flipped back to in_progress"
    );
    assert_eq!(after.provider_batch_id.as_deref(), Some("msgbatch_race"));
}

/// Releasing a claim after a failed submission reverts `submitting → created`
/// (with the error recorded) so the job is retried, and never touches a job that
/// a cancellation has already moved on.
#[tokio::test]
async fn release_batch_submission_reverts_to_created() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("release", "Release"))
        .await
        .unwrap();
    let job = store
        .create_batch_job(NewBatchJob {
            tenant_id: tenant.id,
            virtual_key_id: None,
            provider: "anthropic".to_owned(),
            items: vec![NewBatchItem {
                custom_id: "only".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                request: json!({ "provider": "anthropic", "model": "claude-opus-4-8" }),
            }],
        })
        .await
        .unwrap();

    assert!(store
        .claim_batch_for_submission(tenant.id, job.id)
        .await
        .unwrap());
    store
        .release_batch_submission(tenant.id, job.id, "submit: upstream 503")
        .await
        .unwrap();
    let reverted = store
        .get_batch_job(tenant.id, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reverted.status, BatchStatus::Created);
    assert_eq!(reverted.error.as_deref(), Some("submit: upstream 503"));

    // It can be claimed again (retry).
    assert!(store
        .claim_batch_for_submission(tenant.id, job.id)
        .await
        .unwrap());
}

/// A rollup splits each group's cost into its batch-tier and interactive
/// portions, so the two spend kinds can be told apart (finding #4).
#[tokio::test]
async fn rollup_splits_batch_and_interactive_cost() {
    let (_pg, store) = setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("split", "Split Tenant"))
        .await
        .unwrap();

    let mut usage = Usage::new();
    usage.input_tokens = Some(100);
    usage.output_tokens = Some(10);

    // Two interactive events (cost 2 + 3 = 5) and one batch event (cost 4) on the
    // same model.
    for (cost, is_batch) in [
        (Decimal::from(2), false),
        (Decimal::from(3), false),
        (Decimal::from(4), true),
    ] {
        store
            .record_event(NewUsageEvent {
                tenant_id: tenant.id,
                virtual_key_id: None,
                conversation_id: None,
                provider: "anthropic".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                usage: usage.clone(),
                cost: Some(cost),
                is_batch,
            })
            .await
            .unwrap();
    }

    let by_model = store
        .rollup_grouped(tenant.id, None, None, RollupGroup::Model)
        .await
        .unwrap();
    let opus = by_model
        .iter()
        .find(|r| r.group.as_deref() == Some("claude-opus-4-8"))
        .unwrap();
    assert_eq!(opus.event_count, 3);
    assert_eq!(opus.cost, Decimal::from(9));
    assert_eq!(opus.batch_cost, Decimal::from(4));
    assert_eq!(opus.interactive_cost, Decimal::from(5));
    assert_eq!(
        opus.batch_cost + opus.interactive_cost,
        opus.cost,
        "batch + interactive must reconstitute the total cost"
    );
}
