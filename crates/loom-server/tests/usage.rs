//! Integration tests for spend tracking: per-turn usage capture, pricing (incl.
//! the cache-read/cache-write split), the tenant and gateway-wide rollup
//! endpoints, price versioning, and the usage-write failure/outbox/drain path.
//!
//! Each test boots a throwaway PostgreSQL 16 container, applies the migrations
//! (which seed current Anthropic prices), builds the real router with a
//! `MockProvider` that reports a chosen [`Usage`], and drives it over HTTP.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::response::Response;
use axum::Router;
use chrono::Utc;
use http::{Request, StatusCode};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use loom_core::{ContentPart, Message, Usage};
use loom_provider::mock::MockProvider;
use loom_provider::{Capability, Provider, StopReason, TurnEvent, TurnEventKind};
use loom_server::{
    build_router, ApiError, AppState, Crypto, KeyHasher, ProviderFactory, UsageRecorder,
};
use loom_store::{
    drain_usage_outbox, NewModelPrice, NewUsageEvent, OutboxStore, PgStore, PricingStore,
};

use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

const ADMIN_TOKEN: &str = "root-admin-secret-token";
const ENC_KEY: [u8; 32] = [0x33; 32];
const PEPPER: &[u8] = b"usage-test-pepper";

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

/// A usage recorder that always routes to the outbox, simulating a primary
/// `usage_events` write failure without a real database fault.
struct OutboxOnlyRecorder;

#[async_trait]
impl UsageRecorder for OutboxOnlyRecorder {
    async fn record(&self, store: &PgStore, event: NewUsageEvent) {
        let _ = store.enqueue_outbox(&event).await;
    }
}

/// Boots a migrated database and returns the container, the store, and a base
/// [`AppState`] whose provider factory always yields `provider`.
async fn setup(provider: Arc<dyn Provider>) -> (ContainerAsync<Postgres>, PgStore, AppState) {
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

async fn create_key(app: &Router, tenant_id: Uuid) -> String {
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
    json_body(resp).await["key"].as_str().unwrap().to_owned()
}

async fn create_conversation(app: &Router, key: &str, model: &str) -> Uuid {
    let resp = send(
        app,
        request(
            "POST",
            "/v1/conversations",
            Some(key),
            Some(json!({ "provider": "anthropic", "model": model })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    Uuid::parse_str(json_body(resp).await["id"].as_str().unwrap()).unwrap()
}

async fn run_turn(app: &Router, key: &str, convo: Uuid, stream: bool) -> StatusCode {
    let resp = send(
        app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(key),
            Some(json!({ "content": [{ "type": "text", "text": "hi" }], "stream": stream })),
        ),
    )
    .await;
    let status = resp.status();
    // Drain the body so a streamed turn's finalizer (usage capture + message
    // persistence) runs to completion — the SSE stream is produced lazily.
    let _ = axum::body::to_bytes(resp.into_body(), usize::MAX).await;
    status
}

/// An assistant message reporting the given token usage, with a cache split.
fn assistant_with_usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Message {
    let mut usage = Usage::new();
    usage.input_tokens = Some(input);
    usage.output_tokens = Some(output);
    usage.cache_read_tokens = Some(cache_read);
    usage.cache_write_tokens = Some(cache_write);
    let mut message = Message::assistant("ok");
    message.usage = Some(usage);
    message
}

/// The `cost` field of a rollup row, as a comparable string (tolerant of
/// whether `rust_decimal` serialises as a JSON string or number).
fn cost_str(row: &Value) -> String {
    let cost = &row["cost"];
    cost.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| cost.to_string())
}

/// A non-streaming turn records one priced usage event including the
/// cache-read/cache-write split, surfaced by both the tenant `/v1/usage`
/// rollup and the gateway-wide `/admin/usage` rollup.
#[tokio::test]
async fn non_streaming_turn_records_priced_usage_with_cache_split() {
    // 1M of each token class against seeded opus (5 / 25 / cache_write 6.25 /
    // cache_read 0.50) => 5 + 25 + 0.50 + 6.25 = 36.75.
    let provider = MockProvider::new("anthropic", "claude-opus-4-8", [Capability::ClientTools])
        .with_completion(assistant_with_usage(
            1_000_000, 1_000_000, 1_000_000, 1_000_000,
        ));
    let (_pg, _store, state) = setup(Arc::new(provider)).await;
    let app = build_router(state);

    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key, "claude-opus-4-8").await;
    assert_eq!(run_turn(&app, &key, convo, false).await, StatusCode::OK);

    // Tenant rollup grouped by model.
    let usage = send(
        &app,
        request("GET", "/v1/usage?group_by=model", Some(&key), None),
    )
    .await;
    assert_eq!(usage.status(), StatusCode::OK);
    let body = json_body(usage).await;
    assert_eq!(body["group_by"], "model");
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["group"], "claude-opus-4-8");
    assert_eq!(row["event_count"], 1);
    assert_eq!(row["input_tokens"], 1_000_000);
    assert_eq!(row["output_tokens"], 1_000_000);
    assert_eq!(row["cache_read_tokens"], 1_000_000);
    assert_eq!(row["cache_write_tokens"], 1_000_000);
    assert!(
        cost_str(row).contains("36.75"),
        "cost was {}",
        cost_str(row)
    );

    // Gateway-wide rollup by tenant (root token).
    let admin = send(
        &app,
        request(
            "GET",
            "/admin/usage?group_by=tenant",
            Some(ADMIN_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(admin.status(), StatusCode::OK);
    let admin_body = json_body(admin).await;
    let admin_rows = admin_body["rows"].as_array().unwrap();
    let mine = admin_rows
        .iter()
        .find(|r| r["group"] == tenant.to_string())
        .expect("tenant present in gateway rollup");
    assert!(cost_str(mine).contains("36.75"));
}

/// A streaming turn finalises usage from the turn-end snapshot and records a
/// priced event including the cache split.
#[tokio::test]
async fn streaming_turn_records_priced_usage() {
    let mut usage = Usage::new();
    usage.input_tokens = Some(2_000_000);
    usage.output_tokens = Some(0);
    usage.cache_read_tokens = Some(1_000_000);
    usage.cache_write_tokens = Some(0);
    // 2M input * 5 + 1M cache_read * 0.50 = 10 + 0.50 = 10.50.
    let events = vec![
        TurnEvent::new(
            TurnEventKind::ContentPartComplete {
                index: 0,
                part: ContentPart::text("hello"),
            },
            json!({ "type": "content_block_stop" }),
        ),
        TurnEvent::new(
            TurnEventKind::TurnEnded {
                stop_reason: StopReason::EndTurn,
                usage: Some(usage),
            },
            json!({ "type": "message_stop" }),
        ),
    ];
    let provider = MockProvider::new("anthropic", "claude-opus-4-8", [Capability::Streaming])
        .with_events(events);
    let (_pg, _store, state) = setup(Arc::new(provider)).await;
    let app = build_router(state);

    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key, "claude-opus-4-8").await;
    assert_eq!(run_turn(&app, &key, convo, true).await, StatusCode::OK);

    let usage = send(
        &app,
        request("GET", "/v1/usage?group_by=model", Some(&key), None),
    )
    .await;
    let body = json_body(usage).await;
    let row = &body["rows"].as_array().unwrap()[0];
    assert_eq!(row["input_tokens"], 2_000_000);
    assert_eq!(row["cache_read_tokens"], 1_000_000);
    assert!(cost_str(row).contains("10.5"), "cost was {}", cost_str(row));
}

/// A new price version affects only turns taken after it becomes effective;
/// events priced under the old version keep their stored cost.
#[tokio::test]
async fn price_versioning_affects_new_events_only() {
    // 1M input + 1M output, no cache. Seeded opus: 5 + 25 = 30.
    let provider = MockProvider::new("anthropic", "claude-opus-4-8", [Capability::ClientTools])
        .with_completion(assistant_with_usage(1_000_000, 1_000_000, 0, 0));
    let (_pg, store, state) = setup(Arc::new(provider)).await;
    let app = build_router(state);

    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;

    // Turn 1 on convo_a, priced under the seeded price (cost 30).
    let convo_a = create_conversation(&app, &key, "claude-opus-4-8").await;
    assert_eq!(run_turn(&app, &key, convo_a, false).await, StatusCode::OK);

    // A new, more expensive price version becomes effective now (10 + 50 = 60).
    store
        .upsert_price(NewModelPrice {
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            input_per_mtok: Decimal::from(10),
            output_per_mtok: Decimal::from(50),
            cache_write_per_mtok: Decimal::from(0),
            cache_read_per_mtok: Decimal::from(0),
            server_tool_prices: json!({}),
            batch_multiplier: rust_decimal::Decimal::ONE,
            currency: "USD".to_owned(),
            effective_from: Utc::now(),
        })
        .await
        .unwrap();

    // Turn 2 on convo_b, priced under the new version (cost 60).
    let convo_b = create_conversation(&app, &key, "claude-opus-4-8").await;
    assert_eq!(run_turn(&app, &key, convo_b, false).await, StatusCode::OK);

    // Per-conversation rollup: the old event kept 30, the new one got 60.
    let usage = send(
        &app,
        request("GET", "/v1/usage?group_by=conversation", Some(&key), None),
    )
    .await;
    let body = json_body(usage).await;
    let rows = body["rows"].as_array().unwrap();
    let a = rows
        .iter()
        .find(|r| r["group"] == convo_a.to_string())
        .unwrap();
    let b = rows
        .iter()
        .find(|r| r["group"] == convo_b.to_string())
        .unwrap();
    assert!(
        cost_str(a).contains("30"),
        "convo_a cost was {}",
        cost_str(a)
    );
    assert!(
        cost_str(b).contains("60"),
        "convo_b cost was {}",
        cost_str(b)
    );
}

/// A usage-write failure must not fail the turn: the event lands in the outbox,
/// the turn still returns 200, and a later drain settles it into `usage_events`.
#[tokio::test]
async fn usage_write_failure_lands_in_outbox_and_drains() {
    let provider = MockProvider::new("anthropic", "claude-opus-4-8", [Capability::ClientTools])
        .with_completion(assistant_with_usage(1_000_000, 1_000_000, 0, 0));
    let (_pg, store, state) = setup(Arc::new(provider)).await;
    // Force the primary write to fail (route straight to the outbox).
    let state = state.with_usage_recorder(Arc::new(OutboxOnlyRecorder));
    let app = build_router(state);

    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key, "claude-opus-4-8").await;

    // The turn succeeds even though the usage write was deferred.
    assert_eq!(run_turn(&app, &key, convo, false).await, StatusCode::OK);

    // Nothing has landed in usage_events yet; it is parked in the outbox.
    let before = send(
        &app,
        request("GET", "/v1/usage?group_by=model", Some(&key), None),
    )
    .await;
    assert!(json_body(before).await["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(store.list_pending_outbox(100).await.unwrap().len(), 1);

    // Draining replays the parked event into usage_events.
    let report = drain_usage_outbox(&store, 100).await.unwrap();
    assert_eq!(report.processed, 1);
    assert_eq!(report.failed, 0);
    assert!(store.list_pending_outbox(100).await.unwrap().is_empty());

    let after = send(
        &app,
        request("GET", "/v1/usage?group_by=model", Some(&key), None),
    )
    .await;
    let body = json_body(after).await;
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["input_tokens"], 1_000_000);
    // The cost was computed at turn time (seeded opus 5 + 25 = 30) and preserved.
    assert!(cost_str(&rows[0]).contains("30"));
}

/// The tenant rollup rejects an unknown `group_by`.
#[tokio::test]
async fn usage_rollup_rejects_unknown_group_by() {
    let provider = MockProvider::new("anthropic", "claude-opus-4-8", [Capability::ClientTools]);
    let (_pg, _store, state) = setup(Arc::new(provider)).await;
    let app = build_router(state);

    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;

    let resp = send(
        &app,
        request("GET", "/v1/usage?group_by=nonsense", Some(&key), None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"]["code"], "bad_request");
}
