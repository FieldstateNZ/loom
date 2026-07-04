//! Integration tests for usage-event recording and rollups, against a real
//! database.

mod common;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::json;

use loom_core::{Conversation, ProviderBinding, Usage};
use loom_store::{
    ConversationStore, KeyStore, NewTenant, NewUsageEvent, NewVirtualKey, Pricer, PricingStore,
    RollupGroup, TenantStore, UsageStore,
};

#[tokio::test]
async fn usage_events_record_and_rollup() {
    let (_pg, store) = common::setup().await;
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
    let (_pg, store) = common::setup().await;
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

/// Grouped rollups sum tokens and cost per key / model / conversation for a
/// tenant, and the gateway-wide rollup groups by tenant.
#[tokio::test]
async fn grouped_rollups_aggregate_per_dimension() {
    let (_pg, store) = common::setup().await;
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

/// A rollup splits each group's cost into its batch-tier and interactive
/// portions, so the two spend kinds can be told apart (finding #4).
#[tokio::test]
async fn rollup_splits_batch_and_interactive_cost() {
    let (_pg, store) = common::setup().await;
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
