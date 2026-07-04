//! Integration tests for tenant CRUD and migrations, against a real database.

mod common;

use uuid::Uuid;

use loom_store::{run_migrations, NewTenant, TenantStore};

#[tokio::test]
async fn migrations_apply_and_are_idempotent() {
    let (_pg, store) = common::setup().await;
    // Running again must be a no-op rather than an error.
    run_migrations(store.pool())
        .await
        .expect("re-running migrations is idempotent");
}

#[tokio::test]
async fn tenant_crud() {
    let (_pg, store) = common::setup().await;

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
