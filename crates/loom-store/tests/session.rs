//! Integration tests for the Session substrate: a conversation's current
//! session, its (empty until migration) lineage, and `SessionStore` reads,
//! against a real database.

mod common;

use loom_core::{Conversation, ProviderBinding, SessionStatus};
use loom_store::{ConversationStore, NewTenant, SessionStore, TenantStore};

#[tokio::test]
async fn conversation_mints_current_session_and_empty_lineage() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("sess", "Sess Tenant"))
        .await
        .unwrap();

    let convo = Conversation::new(
        tenant.id,
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    let session_id = convo
        .current_session_id
        .expect("a new conversation mints an active session");
    store.create_conversation(&convo).await.unwrap();

    let reloaded = store
        .get_conversation(tenant.id, convo.id)
        .await
        .unwrap()
        .expect("conversation exists");
    assert_eq!(reloaded.current_session_id, Some(session_id));
    assert!(
        reloaded.previous_session_ids.is_empty(),
        "lineage is empty until the conversation migrates"
    );

    let session = store
        .get_session(tenant.id, session_id)
        .await
        .unwrap()
        .expect("session exists");
    assert_eq!(session.id, session_id);
    assert_eq!(session.conversation_id, convo.id);
    assert_eq!(session.status, SessionStatus::Active);

    let sessions = store.list_sessions(tenant.id, convo.id).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, session_id);
}

#[tokio::test]
async fn sessions_are_tenant_scoped() {
    let (_pg, store) = common::setup().await;
    let tenant_a = store
        .create_tenant(NewTenant::new("sess-a", "A"))
        .await
        .unwrap();
    let tenant_b = store
        .create_tenant(NewTenant::new("sess-b", "B"))
        .await
        .unwrap();

    let convo = Conversation::new(tenant_a.id, ProviderBinding::new("anthropic", "m"));
    let session_id = convo.current_session_id.unwrap();
    store.create_conversation(&convo).await.unwrap();

    assert!(store
        .get_session(tenant_a.id, session_id)
        .await
        .unwrap()
        .is_some());
    assert!(
        store
            .get_session(tenant_b.id, session_id)
            .await
            .unwrap()
            .is_none(),
        "cross-tenant session read must return nothing"
    );
    assert!(store
        .list_sessions(tenant_b.id, convo.id)
        .await
        .unwrap()
        .is_empty());
}
