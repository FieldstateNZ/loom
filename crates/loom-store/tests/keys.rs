//! Integration tests for virtual API key CRUD, against a real database.

mod common;

use rust_decimal::Decimal;
use uuid::Uuid;

use loom_store::{
    BudgetAction, BudgetWindow, KeyBudget, KeyStore, NewTenant, NewVirtualKey, TenantStore,
};

#[tokio::test]
async fn virtual_key_crud() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("keys", "Keys Tenant"))
        .await
        .unwrap();

    let key = store
        .create_key(NewVirtualKey {
            tenant_id: tenant.id,
            key_hash: "hash-abc".to_owned(),
            key_prefix: "sk-live-abc".to_owned(),
            name: "prod".to_owned(),
            scopes: vec!["chat".to_owned(), "usage:read".to_owned()],
            budget: Some(KeyBudget {
                limit_amount: Decimal::new(2500, 2),
                window: BudgetWindow::Monthly,
                action: BudgetAction::Block,
            }),
        })
        .await
        .unwrap();
    assert_eq!(key.status, "active");
    assert_eq!(key.scopes, vec!["chat", "usage:read"]);
    assert_eq!(
        key.budget.as_ref().unwrap().limit_amount,
        Decimal::new(2500, 2)
    );
    assert!(key.last_used_at.is_none());

    let fetched = store.get_key_by_hash("hash-abc").await.unwrap().unwrap();
    assert_eq!(fetched, key);

    assert!(store.touch_key_last_used(key.id).await.unwrap());
    let touched = store.get_key_by_hash("hash-abc").await.unwrap().unwrap();
    assert!(touched.last_used_at.is_some());

    assert!(store.revoke_key(key.id).await.unwrap());
    let revoked = store.get_key_by_hash("hash-abc").await.unwrap().unwrap();
    assert_eq!(revoked.status, "revoked");

    assert!(store.get_key_by_hash("missing").await.unwrap().is_none());
    assert!(!store.revoke_key(Uuid::new_v4()).await.unwrap());
}
