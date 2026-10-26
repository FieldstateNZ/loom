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

#[tokio::test]
async fn upgrades_pre_action_database_without_losing_existing_tenants() {
    let (_pg, store) = common::setup_unmigrated().await;
    let mut previous = sqlx::migrate!("./migrations");
    previous.migrations = std::borrow::Cow::Owned(
        previous
            .iter()
            .filter(|m| m.version < 20261027000000)
            .cloned()
            .collect(),
    );
    previous
        .run(store.pool())
        .await
        .expect("previous release schema");
    let missing: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.gateway_actions')::text")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert!(
        missing.is_none(),
        "The upgrade must begin before the task migration"
    );
    let tenant = store
        .create_tenant(NewTenant::new("upgrade-survivor", "Retained identity"))
        .await
        .unwrap();
    run_migrations(store.pool())
        .await
        .expect("upgrade current embedded release");
    run_migrations(store.pool())
        .await
        .expect("second upgrade is idempotent");
    assert_eq!(store.get_tenant(tenant.id).await.unwrap(), Some(tenant));
    let table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.gateway_actions')::text")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert!(table.is_some());
    let applied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE version=20261027000000 AND success",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(applied, 1);
}
