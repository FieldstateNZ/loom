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
