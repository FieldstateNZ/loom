//! Integration tests for MCP server registration CRUD and tenant isolation,
//! against a real database.

mod common;

use serde_json::json;

use loom_store::{McpServerStore, NewMcpServer, NewTenant, TenantStore};

#[tokio::test]
async fn mcp_server_upsert_get_list_delete_and_tenant_isolation() {
    let (_pg, store) = common::setup().await;
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

#[tokio::test]
async fn private_mcp_leases_serialize_replicas_fence_static_replacement_and_cascade_erasure() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("private-mcp", "Private"))
        .await
        .unwrap();
    let other = store
        .create_tenant(NewTenant::new("private-mcp-other", "Other"))
        .await
        .unwrap();
    let registration = NewMcpServer {
        tenant_id: tenant.id,
        name: "lucidbrain".into(),
        url: "https://memory.invalid/mcp".into(),
        encrypted_token: None,
        nonce: None,
        tool_configuration: None,
    };
    store.upsert_mcp_server(registration.clone()).await.unwrap();
    let mut lease = store
        .lease_mcp_credential(tenant.id, "lucidbrain", true)
        .await
        .unwrap()
        .unwrap();
    assert!(store
        .lease_mcp_credential(tenant.id, "lucidbrain", false)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .lease_mcp_credential(tenant.id, "lucidbrain", true)
        .await
        .unwrap()
        .is_none());
    let separate = store
        .lease_mcp_credential(other.id, "lucidbrain", true)
        .await
        .unwrap()
        .unwrap();
    separate.release().await.unwrap();
    assert_eq!(
        lease
            .save(None, b"opaque ciphertext", &[1; 12])
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        lease
            .save(None, b"not a replacement", &[2; 12])
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        lease
            .save(Some(9), b"wrong generation", &[2; 12])
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        lease
            .save(Some(1), b"next ciphertext", &[2; 12])
            .await
            .unwrap(),
        Some(2)
    );
    assert!(store
        .has_private_mcp_credential(tenant.id, "lucidbrain")
        .await
        .unwrap());
    assert!(!store
        .has_private_mcp_credential(other.id, "lucidbrain")
        .await
        .unwrap());
    lease.release().await.unwrap();
    let mut shared = store
        .lease_mcp_credential(tenant.id, "lucidbrain", false)
        .await
        .unwrap()
        .unwrap();
    let another = store
        .lease_mcp_credential(tenant.id, "lucidbrain", false)
        .await
        .unwrap()
        .unwrap();
    assert!(shared
        .save(Some(2), b"shared writer forbidden", &[3; 12])
        .await
        .is_err());
    assert_eq!(shared.load().await.unwrap().unwrap().0, 2);
    assert!(store
        .delete_mcp_server(tenant.id, "lucidbrain")
        .await
        .is_err());
    shared.release().await.unwrap();
    another.release().await.unwrap();
    assert!(store.upsert_mcp_server(registration).await.is_err());
    assert!(store
        .delete_mcp_server(tenant.id, "lucidbrain")
        .await
        .unwrap());
    assert!(!store
        .has_private_mcp_credential(tenant.id, "lucidbrain")
        .await
        .unwrap());
}

#[tokio::test]
async fn completed_registration_and_lease_release_allow_immediate_exclusive_reacquisition() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new(
            "private-release-ack",
            "Release acknowledgement",
        ))
        .await
        .unwrap();
    let registration = NewMcpServer {
        tenant_id: tenant.id,
        name: "lucidbrain".into(),
        url: "https://memory.invalid/mcp".into(),
        encrypted_token: None,
        nonce: None,
        tool_configuration: None,
    };
    for _ in 0..64 {
        store.upsert_mcp_server(registration.clone()).await.unwrap();
        let lease = store
            .lease_mcp_credential(tenant.id, "lucidbrain", true)
            .await
            .unwrap()
            .expect("a completed registration must release its remote lock");
        lease.release().await.unwrap();
        let lease = store
            .lease_mcp_credential(tenant.id, "lucidbrain", true)
            .await
            .unwrap()
            .expect("release must await PostgreSQL unlock acknowledgement");
        lease.release().await.unwrap();
    }
}
