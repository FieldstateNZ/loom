//! Integration tests for budgets and rate limits (#10).
//!
//! Each test boots a throwaway PostgreSQL 16 container, applies the migrations
//! (which seed current Anthropic prices), and drives the real router with a
//! `MockProvider`. Spend is seeded directly through the #9 usage store so a
//! budget can be pushed over its limit without running many turns.
//!
//! Coverage:
//! - a `block`-action budget over its limit rejects the next turn with `402`
//!   and a `budget_exceeded` envelope;
//! - a `warn`-action budget over its soft limit allows the turn (`200`) and
//!   sets `x-loom-budget-warning`;
//! - a per-key requests-per-minute limit rejects with `429` + `Retry-After`;
//! - a key-level budget overrides the tenant-level budget.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::response::Response;
use axum::Router;
use http::{Request, StatusCode};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use loom_core::{Message, Usage};
use loom_provider::mock::MockProvider;
use loom_provider::{Capability, Provider};
use loom_server::{build_router, ApiError, AppState, Crypto, KeyHasher, ProviderFactory};
use loom_store::{NewUsageEvent, PgStore, UsageStore};

use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

const ADMIN_TOKEN: &str = "root-admin-secret-token";
const ENC_KEY: [u8; 32] = [0x44; 32];
const PEPPER: &[u8] = b"budget-test-pepper";
const MODEL: &str = "claude-opus-4-8";

/// A test factory that returns a fixed provider for any provider name.
struct AnyFactory(Arc<dyn Provider>);

#[async_trait]
impl ProviderFactory for AnyFactory {
    async fn provider(
        &self,
        _state: &AppState,
        _tenant_id: Uuid,
        _provider: &str,
    ) -> Result<Arc<dyn Provider>, ApiError> {
        Ok(self.0.clone())
    }
}

/// A cheap assistant completion carrying a small, fixed usage.
fn cheap_completion() -> Message {
    let mut usage = Usage::new();
    usage.input_tokens = Some(10);
    usage.output_tokens = Some(10);
    let mut message = Message::assistant("ok");
    message.usage = Some(usage);
    message
}

/// Boots a migrated database and a base [`AppState`] whose provider factory
/// always yields a cheap-completion mock.
async fn setup() -> (ContainerAsync<Postgres>, PgStore, AppState) {
    let container = Postgres::default()
        .with_tag("16")
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("map postgres port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let store = PgStore::connect(&url).await.expect("connect to postgres");
    loom_store::run_migrations(store.pool())
        .await
        .expect("migrations apply cleanly");

    let provider: Arc<dyn Provider> = Arc::new(
        MockProvider::new("anthropic", MODEL, [Capability::ClientTools])
            .with_completion(cheap_completion()),
    );
    let state = AppState::new(
        store.clone(),
        Crypto::new(ENC_KEY),
        KeyHasher::new(PEPPER.to_vec()),
        ADMIN_TOKEN.to_owned(),
    )
    .with_provider_factory(Arc::new(AnyFactory(provider)));
    (container, store, state)
}

async fn send(app: &Router, req: Request<Body>) -> Response {
    app.clone().oneshot(req).await.expect("router handled")
}

async fn json_body(resp: Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("body is JSON")
}

fn request(method: &str, uri: &str, bearer: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header(http::header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = match body {
        Some(v) => {
            builder = builder.header(http::header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&v).unwrap())
        }
        None => Body::empty(),
    };
    builder.body(body).unwrap()
}

async fn create_tenant(app: &Router, slug: &str) -> Uuid {
    let resp = send(
        app,
        request(
            "POST",
            "/admin/tenants",
            Some(ADMIN_TOKEN),
            Some(json!({ "slug": slug, "name": slug })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    Uuid::parse_str(json_body(resp).await["id"].as_str().unwrap()).unwrap()
}

/// Mints a key and returns `(plaintext_key, key_id)`.
async fn create_key(app: &Router, tenant_id: Uuid) -> (String, Uuid) {
    let resp = send(
        app,
        request(
            "POST",
            &format!("/admin/tenants/{tenant_id}/keys"),
            Some(ADMIN_TOKEN),
            Some(json!({ "name": "primary" })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = json_body(resp).await;
    (
        body["key"].as_str().unwrap().to_owned(),
        Uuid::parse_str(body["id"].as_str().unwrap()).unwrap(),
    )
}

async fn create_conversation(app: &Router, key: &str) -> Uuid {
    let resp = send(
        app,
        request(
            "POST",
            "/v1/conversations",
            Some(key),
            Some(json!({ "provider": "anthropic", "model": MODEL })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    Uuid::parse_str(json_body(resp).await["id"].as_str().unwrap()).unwrap()
}

/// Runs a non-streaming turn and returns the raw response (headers intact).
async fn run_turn(app: &Router, key: &str, convo: Uuid) -> Response {
    send(
        app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(key),
            Some(json!({ "content": [{ "type": "text", "text": "hi" }] })),
        ),
    )
    .await
}

/// Seeds a priced usage event so a scope's current-window spend jumps by `cost`.
async fn seed_spend(store: &PgStore, tenant_id: Uuid, key_id: Option<Uuid>, cost: i64) {
    let mut usage = Usage::new();
    usage.input_tokens = Some(1);
    store
        .record_event(NewUsageEvent {
            tenant_id,
            virtual_key_id: key_id,
            conversation_id: None,
            provider: "anthropic".to_owned(),
            model: MODEL.to_owned(),
            usage,
            cost: Some(Decimal::from(cost)),
        })
        .await
        .expect("seed usage event");
}

/// Sets a budget on a key via the admin API.
async fn set_key_budget(app: &Router, key_id: Uuid, limit: i64, window: &str, action: &str) {
    let resp = send(
        app,
        request(
            "PUT",
            &format!("/admin/keys/{key_id}/budget"),
            Some(ADMIN_TOKEN),
            Some(json!({ "limit_amount": limit, "window": window, "action": action })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

/// Sets a budget on a tenant via the admin API.
async fn set_tenant_budget(app: &Router, tenant_id: Uuid, limit: i64, window: &str, action: &str) {
    let resp = send(
        app,
        request(
            "PUT",
            &format!("/admin/tenants/{tenant_id}/budget"),
            Some(ADMIN_TOKEN),
            Some(json!({ "limit_amount": limit, "window": window, "action": action })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

/// A `block`-action budget already over its limit rejects the next turn with a
/// `402` and a `budget_exceeded` envelope carrying the breakdown.
#[tokio::test]
async fn block_budget_over_limit_rejects_turn() {
    let (_pg, store, state) = setup().await;
    let app = build_router(state);

    let tenant = create_tenant(&app, "acme").await;
    let (key, key_id) = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key).await;

    // Budget of 20 (total window, block). Seed 25 of spend on the key.
    set_key_budget(&app, key_id, 20, "total", "block").await;
    seed_spend(&store, tenant, Some(key_id), 25).await;

    let resp = run_turn(&app, &key, convo).await;
    assert_eq!(resp.status(), StatusCode::PAYMENT_REQUIRED);
    let body = json_body(resp).await;
    assert_eq!(body["error"]["code"], "budget_exceeded");
    let details = &body["error"]["details"];
    assert_eq!(details["scope"], "key");
    assert_eq!(details["window"], "total");
    // limit and spent are present (serialised as strings or numbers).
    assert!(details.get("limit").is_some());
    assert!(details.get("spent").is_some());
}

/// A `warn`-action budget over its soft limit allows the turn (`200`) but sets
/// the `x-loom-budget-warning` response header.
#[tokio::test]
async fn warn_budget_over_limit_allows_turn_with_header() {
    let (_pg, store, state) = setup().await;
    let app = build_router(state);

    let tenant = create_tenant(&app, "acme").await;
    let (key, key_id) = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key).await;

    set_key_budget(&app, key_id, 20, "total", "warn").await;
    seed_spend(&store, tenant, Some(key_id), 25).await;

    let resp = run_turn(&app, &key, convo).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let warning = resp
        .headers()
        .get("x-loom-budget-warning")
        .expect("budget warning header present")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(warning.contains("key budget"), "header was {warning}");
    // Drain the body.
    let _ = axum::body::to_bytes(resp.into_body(), usize::MAX).await;
}

/// A turn under the budget proceeds cleanly with no warning header.
#[tokio::test]
async fn under_budget_turn_has_no_warning() {
    let (_pg, store, state) = setup().await;
    let app = build_router(state);

    let tenant = create_tenant(&app, "acme").await;
    let (key, key_id) = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key).await;

    set_key_budget(&app, key_id, 100, "total", "block").await;
    seed_spend(&store, tenant, Some(key_id), 5).await;

    let resp = run_turn(&app, &key, convo).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get("x-loom-budget-warning").is_none());
    let _ = axum::body::to_bytes(resp.into_body(), usize::MAX).await;
}

/// A per-key requests-per-minute limit admits up to the cap, then rejects with
/// `429` and a `Retry-After` header.
#[tokio::test]
async fn requests_per_minute_limit_rejects_with_retry_after() {
    let (_pg, _store, state) = setup().await;
    let app = build_router(state);

    let tenant = create_tenant(&app, "acme").await;
    let (key, key_id) = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key).await;

    // Allow only 2 requests/minute.
    let resp = send(
        &app,
        request(
            "PUT",
            &format!("/admin/keys/{key_id}/rate-limit"),
            Some(ADMIN_TOKEN),
            Some(json!({ "requests_per_min": 2 })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // The first two turns are admitted.
    for _ in 0..2 {
        let resp = run_turn(&app, &key, convo).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(resp.into_body(), usize::MAX).await;
    }
    // The third trips the limit.
    let resp = run_turn(&app, &key, convo).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = resp
        .headers()
        .get(http::header::RETRY_AFTER)
        .expect("retry-after header present")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        retry_after.parse::<u64>().is_ok(),
        "retry-after was {retry_after}"
    );
    let body = json_body(resp).await;
    assert_eq!(body["error"]["code"], "rate_limited");
}

/// A key-level budget overrides the tenant-level budget: with the key under its
/// own (higher) limit but the tenant over its (lower) limit, the key budget
/// wins and the turn proceeds.
#[tokio::test]
async fn key_budget_overrides_tenant_budget() {
    let (_pg, store, state) = setup().await;
    let app = build_router(state);

    let tenant = create_tenant(&app, "acme").await;
    let (key, key_id) = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key).await;

    // Tenant budget: block at 10. Key budget: block at 1000 (its own budget).
    set_tenant_budget(&app, tenant, 10, "total", "block").await;
    set_key_budget(&app, key_id, 1000, "total", "block").await;

    // Seed 50 of spend on the key: over the tenant's 10, under the key's 1000.
    seed_spend(&store, tenant, Some(key_id), 50).await;

    // The key budget governs (it overrides the tenant), so the turn proceeds.
    let resp = run_turn(&app, &key, convo).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "key budget should override the tenant budget and allow the turn"
    );
    let _ = axum::body::to_bytes(resp.into_body(), usize::MAX).await;

    // Sanity: clearing the key budget lets the (exceeded) tenant budget bite.
    let resp = send(
        &app,
        request(
            "DELETE",
            &format!("/admin/keys/{key_id}/budget"),
            Some(ADMIN_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = run_turn(&app, &key, convo).await;
    assert_eq!(
        resp.status(),
        StatusCode::PAYMENT_REQUIRED,
        "with the key override gone, the tenant budget blocks"
    );
    let body = json_body(resp).await;
    assert_eq!(body["error"]["details"]["scope"], "tenant");
}
