//! Integration tests for tenant/key budgets, rate limits, and scoped spend,
//! against a real database.

mod common;

use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

use loom_core::Usage;
use loom_store::{
    BudgetAction, BudgetStore, BudgetWindow, KeyBudget, KeyStore, NewTenant, NewUsageEvent,
    NewVirtualKey, RateLimit, TenantStore, UsageStore,
};

/// Tenant and key budgets round-trip through the store, and `budget_spend`
/// sums recorded cost scoped to the tenant or a single key over a time window.
#[tokio::test]
async fn budgets_and_spend_scope_correctly() {
    let (_pg, store) = common::setup().await;
    let tenant = store
        .create_tenant(NewTenant::new("bud", "Budget Tenant"))
        .await
        .unwrap();

    // No budget on a fresh tenant.
    assert!(store.get_tenant_budget(tenant.id).await.unwrap().is_none());

    // Set, read back, then clear the tenant budget.
    let budget = KeyBudget {
        limit_amount: Decimal::from(50),
        window: BudgetWindow::Monthly,
        action: BudgetAction::Warn,
    };
    assert!(store
        .set_tenant_budget(tenant.id, Some(budget.clone()))
        .await
        .unwrap());
    assert_eq!(
        store.get_tenant_budget(tenant.id).await.unwrap(),
        Some(budget)
    );
    assert!(store.set_tenant_budget(tenant.id, None).await.unwrap());
    assert!(store.get_tenant_budget(tenant.id).await.unwrap().is_none());
    // A missing tenant reports no update.
    assert!(!store.set_tenant_budget(Uuid::new_v4(), None).await.unwrap());

    let key = store
        .create_key(NewVirtualKey {
            tenant_id: tenant.id,
            key_hash: "hash-bud".to_owned(),
            key_prefix: "loom_bud".to_owned(),
            name: "bud".to_owned(),
            scopes: Vec::new(),
            budget: None,
        })
        .await
        .unwrap();

    // Key budget + rate limit round-trip through get_key_by_hash.
    assert!(store
        .set_key_budget(
            key.id,
            Some(KeyBudget {
                limit_amount: Decimal::from(10),
                window: BudgetWindow::Daily,
                action: BudgetAction::Block,
            }),
        )
        .await
        .unwrap());
    assert!(store
        .set_key_rate_limit(
            key.id,
            Some(RateLimit {
                requests_per_min: Some(30),
                tokens_per_min: Some(90_000),
            }),
        )
        .await
        .unwrap());
    let fetched = store.get_key_by_hash("hash-bud").await.unwrap().unwrap();
    assert_eq!(fetched.budget.unwrap().limit_amount, Decimal::from(10));
    assert_eq!(fetched.rate_limit.unwrap().requests_per_min, Some(30));

    // Seed spend: the key spends 7, a second (keyless) event spends 5.
    let mut usage = Usage::new();
    usage.input_tokens = Some(1);
    store
        .record_event(NewUsageEvent {
            tenant_id: tenant.id,
            virtual_key_id: Some(key.id),
            conversation_id: None,
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            usage: usage.clone(),
            cost: Some(Decimal::from(7)),
            is_batch: false,
        })
        .await
        .unwrap();
    store
        .record_event(NewUsageEvent {
            tenant_id: tenant.id,
            virtual_key_id: None,
            conversation_id: None,
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            usage,
            cost: Some(Decimal::from(5)),
            is_batch: false,
        })
        .await
        .unwrap();

    // Tenant-wide spend sees both events (12); key-scoped spend sees only 7.
    assert_eq!(
        store.budget_spend(tenant.id, None, None).await.unwrap(),
        Decimal::from(12)
    );
    assert_eq!(
        store
            .budget_spend(tenant.id, Some(key.id), None)
            .await
            .unwrap(),
        Decimal::from(7)
    );
    // A future lower bound excludes the just-recorded events.
    let future = Utc::now() + chrono::Duration::hours(1);
    assert_eq!(
        store
            .budget_spend(tenant.id, None, Some(future))
            .await
            .unwrap(),
        Decimal::ZERO
    );
}
