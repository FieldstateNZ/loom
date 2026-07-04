//! Integration tests for the usage outbox drain path, against a real
//! database.

mod common;

use rust_decimal::Decimal;
use uuid::Uuid;

use loom_core::Usage;
use loom_store::{
    drain_usage_outbox, NewTenant, NewUsageEvent, OutboxStore, TenantStore, UsageStore,
};

/// The outbox parks a usage event and the drain path replays it into
/// `usage_events`; an event that keeps failing stays pending with a bumped
/// attempt count.
#[tokio::test]
async fn outbox_enqueue_and_drain() {
    let (_pg, store) = common::setup().await;
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
