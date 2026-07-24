//! Integration tests for the normalised session event log, against a real
//! database.

mod common;

use loom_core::{Conversation, EventKind, ProviderBinding, RunStatus};
use loom_store::{ConversationStore, NewTenant, SessionEventStore, TenantStore};

#[tokio::test]
async fn events_append_in_order_and_paginate_by_cursor() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("ev", "Ev Tenant"))
        .await
        .unwrap();
    let convo = Conversation::new(tenant.id, ProviderBinding::new("anthropic", "m"));
    let session_id = convo.current_session_id.unwrap();
    store.create_conversation(&convo).await.unwrap();

    let e0 = store
        .append_event(
            tenant.id,
            session_id,
            &EventKind::Status {
                status: RunStatus::Running,
            },
        )
        .await
        .unwrap()
        .unwrap();
    let e1 = store
        .append_event(
            tenant.id,
            session_id,
            &EventKind::AssistantMessageStart {
                message_id: "m1".to_owned(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let e2 = store
        .append_event(
            tenant.id,
            session_id,
            &EventKind::Status {
                status: RunStatus::Idle,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        e0.id < e1.id && e1.id < e2.id,
        "event ids must be lexicographically monotonic"
    );

    let all = store
        .list_session_events(tenant.id, session_id, None, 100)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].id, e0.id);
    assert_eq!(all[2].id, e2.id);

    let after_first = store
        .list_session_events(tenant.id, session_id, Some(&e0.id), 100)
        .await
        .unwrap();
    assert_eq!(after_first.len(), 2);
    assert_eq!(after_first[0].id, e1.id);
}

#[tokio::test]
async fn events_are_tenant_scoped() {
    let (_pg, store) = common::setup().await;
    let tenant_a = store
        .create_tenant(NewTenant::new("ev-a", "A"))
        .await
        .unwrap();
    let tenant_b = store
        .create_tenant(NewTenant::new("ev-b", "B"))
        .await
        .unwrap();
    let convo = Conversation::new(tenant_a.id, ProviderBinding::new("anthropic", "m"));
    let session_id = convo.current_session_id.unwrap();
    store.create_conversation(&convo).await.unwrap();

    assert!(
        store
            .append_event(
                tenant_b.id,
                session_id,
                &EventKind::Status {
                    status: RunStatus::Idle
                }
            )
            .await
            .unwrap()
            .is_none(),
        "cross-tenant append must no-op"
    );
    store
        .append_event(
            tenant_a.id,
            session_id,
            &EventKind::Status {
                status: RunStatus::Idle,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(store
        .list_session_events(tenant_b.id, session_id, None, 100)
        .await
        .unwrap()
        .is_empty());
}
