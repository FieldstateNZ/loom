//! Integration tests for the pending tool-call model and `sendToolResult`
//! correlation, against a real database.

mod common;

use serde_json::json;

use loom_core::{Conversation, ProviderBinding, ToolKind};
use loom_store::{
    ConversationStore, NewPendingToolCall, NewTenant, PendingToolCallStore, ResolveOutcome,
    TenantStore,
};

fn call(tool_use_id: &str, kind: ToolKind, name: &str) -> NewPendingToolCall {
    NewPendingToolCall {
        tool_use_id: tool_use_id.to_owned(),
        kind,
        name: name.to_owned(),
        input: json!({}),
        mcp_server_url: None,
    }
}

#[tokio::test]
async fn record_list_resolve_lifecycle() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("ptc", "PTC Tenant"))
        .await
        .unwrap();
    let convo = Conversation::new(tenant.id, ProviderBinding::new("anthropic", "m"));
    let session_id = convo.current_session_id.unwrap();
    store.create_conversation(&convo).await.unwrap();

    store
        .record_pending(
            tenant.id,
            session_id,
            call("tu_1", ToolKind::Custom, "get_weather"),
        )
        .await
        .unwrap()
        .expect("recorded");
    store
        .record_pending(
            tenant.id,
            session_id,
            call("tu_2", ToolKind::Builtin, "web_search"),
        )
        .await
        .unwrap()
        .expect("recorded");

    let pending = store.list_pending(tenant.id, session_id).await.unwrap();
    assert_eq!(pending.len(), 2);

    // resolve tu_1 -> Resolved; the pending set drops to just tu_2.
    assert_eq!(
        store
            .resolve(tenant.id, session_id, "tu_1", json!({ "ok": true }), false)
            .await
            .unwrap(),
        ResolveOutcome::Resolved
    );
    let pending = store.list_pending(tenant.id, session_id).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tool_use_id, "tu_2");

    // A duplicate result for tu_1 is rejected as AlreadyResolved.
    assert_eq!(
        store
            .resolve(tenant.id, session_id, "tu_1", json!({}), false)
            .await
            .unwrap(),
        ResolveOutcome::AlreadyResolved
    );

    // An unknown tool_use_id is rejected as NotFound.
    assert_eq!(
        store
            .resolve(tenant.id, session_id, "nope", json!({}), false)
            .await
            .unwrap(),
        ResolveOutcome::NotFound
    );
}

#[tokio::test]
async fn pending_calls_are_tenant_scoped() {
    let (_pg, store) = common::setup().await;
    let tenant_a = store
        .create_tenant(NewTenant::new("ptc-a", "A"))
        .await
        .unwrap();
    let tenant_b = store
        .create_tenant(NewTenant::new("ptc-b", "B"))
        .await
        .unwrap();
    let convo = Conversation::new(tenant_a.id, ProviderBinding::new("anthropic", "m"));
    let session_id = convo.current_session_id.unwrap();
    store.create_conversation(&convo).await.unwrap();
    store
        .record_pending(tenant_a.id, session_id, call("tu_1", ToolKind::Custom, "t"))
        .await
        .unwrap()
        .unwrap();

    // Tenant B cannot record against, list, or resolve tenant A's session.
    assert!(store
        .record_pending(tenant_b.id, session_id, call("tu_x", ToolKind::Custom, "t"))
        .await
        .unwrap()
        .is_none());
    assert!(store
        .list_pending(tenant_b.id, session_id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .resolve(tenant_b.id, session_id, "tu_1", json!({}), false)
            .await
            .unwrap(),
        ResolveOutcome::NotFound
    );
}
