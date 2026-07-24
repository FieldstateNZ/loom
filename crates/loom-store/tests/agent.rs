//! Integration tests for the AgentDefinition registry and publish semantics,
//! against a real database.

mod common;

use loom_core::AgentContent;
use loom_store::{AgentStore, NewAgentDefinition, NewTenant, TenantStore};

#[tokio::test]
async fn create_publish_edit_publish_flow() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("agent", "Agent Tenant"))
        .await
        .unwrap();

    let def = store
        .create_definition(
            tenant.id,
            NewAgentDefinition::new(
                "assistant",
                AgentContent::new("anthropic", "claude-opus-4-8"),
            ),
        )
        .await
        .unwrap();
    assert_eq!(def.draft_version, 1);
    assert_eq!(def.published_version, None);

    // The v1 snapshot is recorded at mint.
    let v1 = store
        .get_version(tenant.id, def.id, 1)
        .await
        .unwrap()
        .expect("v1 exists");
    assert_eq!(v1.content.model, "claude-opus-4-8");
    assert_eq!(v1.content.instructions, None);

    // publish -> published 1.
    assert_eq!(store.publish(tenant.id, def.id).await.unwrap(), Some(1));

    // edit -> draft 2 + a new snapshot; the v1 snapshot stays immutable.
    let mut v2_content = AgentContent::new("anthropic", "claude-opus-4-8");
    v2_content.instructions = Some("Be concise.".to_owned());
    assert_eq!(
        store
            .update_definition(tenant.id, def.id, v2_content)
            .await
            .unwrap(),
        Some(2)
    );
    let v1_again = store
        .get_version(tenant.id, def.id, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        v1_again.content.instructions, None,
        "v1 snapshot is immutable"
    );
    let v2 = store
        .get_version(tenant.id, def.id, 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v2.content.instructions.as_deref(), Some("Be concise."));

    // The published pointer stays at 1 until a re-publish.
    let after_edit = store
        .get_definition(tenant.id, def.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_edit.published_version, Some(1));
    assert_eq!(after_edit.draft_version, 2);

    // publish -> published 2 (monotonic).
    assert_eq!(store.publish(tenant.id, def.id).await.unwrap(), Some(2));
}

#[tokio::test]
async fn definitions_are_tenant_scoped() {
    let (_pg, store) = common::setup().await;
    let tenant_a = store
        .create_tenant(NewTenant::new("agent-a", "A"))
        .await
        .unwrap();
    let tenant_b = store
        .create_tenant(NewTenant::new("agent-b", "B"))
        .await
        .unwrap();

    let def = store
        .create_definition(
            tenant_a.id,
            NewAgentDefinition::new("x", AgentContent::new("anthropic", "m")),
        )
        .await
        .unwrap();

    assert!(store
        .get_definition(tenant_b.id, def.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store.publish(tenant_b.id, def.id).await.unwrap(),
        None,
        "cross-tenant publish must no-op"
    );
    assert!(store
        .update_definition(tenant_b.id, def.id, AgentContent::new("anthropic", "m"))
        .await
        .unwrap()
        .is_none());
    assert!(store
        .get_version(tenant_b.id, def.id, 1)
        .await
        .unwrap()
        .is_none());
}
