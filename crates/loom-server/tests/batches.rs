//! Integration tests for the asynchronous batch API and poll worker.
//!
//! Each test boots a throwaway PostgreSQL 16 container, applies the migrations
//! (which seed current Anthropic prices, including the 0.5 batch multiplier),
//! builds the real router, and drives the full lifecycle
//! (`created → in_progress → ended`) by calling
//! [`run_batch_poll_pass`](loom_server::run_batch_poll_pass) directly — so the
//! test controls time rather than waiting on a real interval. The provider batch
//! surface is a deterministic in-memory fake injected through
//! [`AppState::with_batch_backend_factory`], so no test touches a live API.

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::response::Response;
use axum::Router;
use http::{Request, StatusCode};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use loom_core::Usage;
use loom_provider::mock::MockProvider;
use loom_provider::{Capability, Provider, ProviderError};
use loom_server::{
    build_router, run_batch_poll_pass, ApiError, AppState, BatchBackend, BatchBackendFactory,
    BatchSubmitItem, Crypto, KeyHasher, ProviderBatchResult, ProviderBatchSnapshot,
    ProviderFactory,
};
use loom_store::{BatchCounts, BatchItemStatus, PgStore};

use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

const ADMIN_TOKEN: &str = "root-admin-secret-token";
const ENC_KEY: [u8; 32] = [0x44; 32];
const PEPPER: &[u8] = b"batch-test-pepper";
const MODEL: &str = "claude-opus-4-8";

/// A provider factory returning a fixed provider for any name.
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

/// Mutable state shared across a fake backend's calls.
#[derive(Default)]
struct FakeState {
    /// The `custom_id`s submitted, echoed back on results.
    custom_ids: Vec<String>,
    /// Whether a cancellation was requested (drives the item outcome).
    canceled: bool,
}

/// A deterministic in-memory [`BatchBackend`]: `submit` records the items and
/// reports `in_progress`; the first `poll` reports `ended`; `results` echoes a
/// success (or cancellation) per item with a fixed usage snapshot; `cancel`
/// flips every item to canceled and reports `ended`.
#[derive(Clone)]
struct FakeBackend {
    state: Arc<Mutex<FakeState>>,
    input_tokens: u64,
    output_tokens: u64,
}

impl FakeBackend {
    fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeState::default())),
            input_tokens,
            output_tokens,
        }
    }

    fn count(&self) -> i32 {
        self.state.lock().unwrap().custom_ids.len() as i32
    }
}

#[async_trait]
impl BatchBackend for FakeBackend {
    async fn submit(
        &self,
        items: Vec<BatchSubmitItem>,
    ) -> Result<ProviderBatchSnapshot, ProviderError> {
        let mut st = self.state.lock().unwrap();
        st.custom_ids = items.into_iter().map(|i| i.custom_id).collect();
        let n = st.custom_ids.len() as i32;
        Ok(ProviderBatchSnapshot {
            provider_batch_id: "msgbatch_fake".to_owned(),
            ended: false,
            counts: BatchCounts {
                processing: n,
                ..BatchCounts::default()
            },
            results_url: None,
        })
    }

    async fn poll(&self, _id: &str) -> Result<ProviderBatchSnapshot, ProviderError> {
        let n = self.count();
        Ok(ProviderBatchSnapshot {
            provider_batch_id: "msgbatch_fake".to_owned(),
            ended: true,
            counts: BatchCounts {
                succeeded: n,
                ..BatchCounts::default()
            },
            results_url: Some("mock://results".to_owned()),
        })
    }

    async fn results(
        &self,
        _snapshot: &ProviderBatchSnapshot,
    ) -> Result<Vec<ProviderBatchResult>, ProviderError> {
        let st = self.state.lock().unwrap();
        let canceled = st.canceled;
        Ok(st
            .custom_ids
            .iter()
            .map(|custom_id| {
                if canceled {
                    ProviderBatchResult {
                        custom_id: custom_id.clone(),
                        outcome: BatchItemStatus::Canceled,
                        result: json!({ "type": "canceled" }),
                        usage: None,
                    }
                } else {
                    let mut usage = Usage::new();
                    usage.input_tokens = Some(self.input_tokens);
                    usage.output_tokens = Some(self.output_tokens);
                    ProviderBatchResult {
                        custom_id: custom_id.clone(),
                        outcome: BatchItemStatus::Succeeded,
                        result: json!({
                            "type": "succeeded",
                            "message": {
                                "role": "assistant",
                                "content": [{ "type": "text", "text": "ok" }]
                            }
                        }),
                        usage: Some(usage),
                    }
                }
            })
            .collect())
    }

    async fn cancel(&self, _id: &str) -> Result<ProviderBatchSnapshot, ProviderError> {
        let mut st = self.state.lock().unwrap();
        st.canceled = true;
        let n = st.custom_ids.len() as i32;
        Ok(ProviderBatchSnapshot {
            provider_batch_id: "msgbatch_fake".to_owned(),
            ended: true,
            counts: BatchCounts {
                canceled: n,
                ..BatchCounts::default()
            },
            results_url: Some("mock://results".to_owned()),
        })
    }
}

/// A batch-backend factory returning a fixed fake backend.
struct FakeBatchFactory(Arc<dyn BatchBackend>);

#[async_trait]
impl BatchBackendFactory for FakeBatchFactory {
    async fn backend(
        &self,
        _state: &AppState,
        _tenant_id: Uuid,
        _provider: &str,
    ) -> Result<Arc<dyn BatchBackend>, ApiError> {
        Ok(self.0.clone())
    }
}

/// Boots a migrated database and returns the container, the shared [`AppState`]
/// (for driving poll passes), and the router.
async fn setup(backend: Arc<dyn BatchBackend>) -> (ContainerAsync<Postgres>, AppState, Router) {
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

    // A provider offering the batch capability, for create-time negotiation.
    let provider: Arc<dyn Provider> = Arc::new(MockProvider::new(
        "anthropic",
        MODEL,
        [Capability::Batches, Capability::Streaming],
    ));

    let state = AppState::new(
        store,
        Crypto::new(ENC_KEY),
        KeyHasher::new(PEPPER.to_vec()),
        ADMIN_TOKEN.to_owned(),
    )
    .with_provider_factory(Arc::new(AnyFactory(provider)))
    .with_batch_backend_factory(Arc::new(FakeBatchFactory(backend)));

    let router = build_router(state.clone());
    (container, state, router)
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

async fn text_body(resp: Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    String::from_utf8(bytes.to_vec()).expect("body is UTF-8")
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

/// A two-item batch create body.
fn two_item_batch() -> Value {
    json!({
        "items": [
            {
                "custom_id": "first",
                "provider": "anthropic",
                "model": MODEL,
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "one" }] }]
            },
            {
                "provider": "anthropic",
                "model": MODEL,
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "two" }] }]
            }
        ]
    })
}

async fn get_status(app: &Router, key: &str, id: &str) -> Value {
    let resp = send(
        app,
        request("GET", &format!("/v1/batches/{id}"), Some(key), None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    json_body(resp).await
}

/// The full lifecycle: create → poll (created→in_progress) → poll
/// (in_progress→ended) → per-item results, with usage priced at the discounted
/// batch tier in the rollup.
#[tokio::test]
async fn batch_lifecycle_create_poll_results_and_batch_pricing() {
    let backend = Arc::new(FakeBackend::new(1_000_000, 1_000_000));
    let (_pg, state, app) = setup(backend).await;
    let tenant = create_tenant(&app, "batch").await;
    let key = create_key(&app, tenant).await;

    // Create — status `created`.
    let resp = send(
        &app,
        request("POST", "/v1/batches", Some(&key), Some(two_item_batch())),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = json_body(resp).await;
    let id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["status"], "created");
    assert_eq!(created["total_items"], 2);

    // First poll pass: submit to the provider → `in_progress`.
    let r1 = run_batch_poll_pass(&state).await;
    assert_eq!(r1.advanced, 1);
    let status = get_status(&app, &key, &id).await;
    assert_eq!(status["status"], "in_progress");
    assert_eq!(status["provider_batch_id"], "msgbatch_fake");
    assert_eq!(status["counts"]["processing"], 2);

    // Second poll pass: provider reports ended → results fetched, usage priced.
    let r2 = run_batch_poll_pass(&state).await;
    assert_eq!(r2.advanced, 1);
    let status = get_status(&app, &key, &id).await;
    assert_eq!(status["status"], "ended");
    assert_eq!(status["counts"]["succeeded"], 2);
    assert!(status["ended_at"].is_string());

    // A third pass is a no-op: the ended job is no longer active.
    let r3 = run_batch_poll_pass(&state).await;
    assert_eq!(r3.advanced, 0);

    // Results — streamed JSONL, one object per item.
    let resp = send(
        &app,
        request(
            "GET",
            &format!("/v1/batches/{id}/results"),
            Some(&key),
            None,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/x-ndjson")
    );
    let body = text_body(resp).await;
    let lines: Vec<Value> = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line is JSON"))
        .collect();
    assert_eq!(lines.len(), 2);
    let ids: Vec<&str> = lines
        .iter()
        .map(|l| l["custom_id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"first"));
    assert!(ids.contains(&"item-1"));
    for line in &lines {
        assert_eq!(line["status"], "succeeded");
        assert_eq!(line["result"]["type"], "succeeded");
    }

    // Usage rollup — priced at the batch (discounted) tier. Per item:
    // (1M×5 + 1M×25) = 30, ×0.5 batch multiplier = 15; two items → 30 total.
    // Interactive pricing would have been 60, so 30 proves the discount applied.
    let resp = send(
        &app,
        request("GET", "/v1/usage?group_by=model", Some(&key), None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let rows = body["rows"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| r["group"] == MODEL)
        .expect("a rollup row for the model");
    let cost = row["cost"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| row["cost"].to_string());
    assert_eq!(
        Decimal::from_str(&cost).unwrap(),
        Decimal::from(30),
        "batch usage should be priced at the discounted tier (got {cost})"
    );
    assert_eq!(row["event_count"], 2);
}

/// Cancelling an in-flight batch: the job moves to `canceling`, and a subsequent
/// poll relays the cancellation and settles the job at `ended` with every item
/// canceled.
#[tokio::test]
async fn batch_cancel_in_flight() {
    let backend = Arc::new(FakeBackend::new(1_000, 1_000));
    let (_pg, state, app) = setup(backend).await;
    let tenant = create_tenant(&app, "cancel").await;
    let key = create_key(&app, tenant).await;

    let resp = send(
        &app,
        request("POST", "/v1/batches", Some(&key), Some(two_item_batch())),
    )
    .await;
    let id = json_body(resp).await["id"].as_str().unwrap().to_owned();

    // Submit to the provider (→ in_progress) then request cancellation.
    run_batch_poll_pass(&state).await;
    let resp = send(
        &app,
        request(
            "POST",
            &format!("/v1/batches/{id}/cancel"),
            Some(&key),
            None,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(json_body(resp).await["status"], "canceling");

    // The next poll pass relays the cancellation and finalises the job.
    let report = run_batch_poll_pass(&state).await;
    assert_eq!(report.advanced, 1);
    let status = get_status(&app, &key, &id).await;
    assert_eq!(status["status"], "ended");
    assert_eq!(status["counts"]["canceled"], 2);

    // Every item is recorded canceled.
    let resp = send(
        &app,
        request(
            "GET",
            &format!("/v1/batches/{id}/results"),
            Some(&key),
            None,
        ),
    )
    .await;
    let body = text_body(resp).await;
    for line in body.lines().filter(|l| !l.trim().is_empty()) {
        let value: Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["status"], "canceled");
    }
}

/// Cancelling a batch that has not yet been submitted finalises it immediately,
/// without ever contacting the provider.
#[tokio::test]
async fn batch_cancel_before_submission() {
    let backend = Arc::new(FakeBackend::new(1, 1));
    let (_pg, state, app) = setup(backend).await;
    let tenant = create_tenant(&app, "precancel").await;
    let key = create_key(&app, tenant).await;

    let resp = send(
        &app,
        request("POST", "/v1/batches", Some(&key), Some(two_item_batch())),
    )
    .await;
    let id = json_body(resp).await["id"].as_str().unwrap().to_owned();

    // Cancel while still `created` — settles immediately as ended/canceled.
    let resp = send(
        &app,
        request(
            "POST",
            &format!("/v1/batches/{id}/cancel"),
            Some(&key),
            None,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["status"], "ended");
    assert_eq!(body["counts"]["canceled"], 2);

    // A poll pass finds nothing active to advance.
    let report = run_batch_poll_pass(&state).await;
    assert_eq!(report.advanced, 0);
}

/// Batches are tenant-scoped: another tenant cannot read a batch it does not own.
#[tokio::test]
async fn batch_is_tenant_scoped() {
    let backend = Arc::new(FakeBackend::new(1, 1));
    let (_pg, _state, app) = setup(backend).await;
    let tenant_a = create_tenant(&app, "owner").await;
    let key_a = create_key(&app, tenant_a).await;
    let tenant_b = create_tenant(&app, "intruder").await;
    let key_b = create_key(&app, tenant_b).await;

    let resp = send(
        &app,
        request("POST", "/v1/batches", Some(&key_a), Some(two_item_batch())),
    )
    .await;
    let id = json_body(resp).await["id"].as_str().unwrap().to_owned();

    // Tenant B cannot see tenant A's batch.
    let resp = send(
        &app,
        request("GET", &format!("/v1/batches/{id}"), Some(&key_b), None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let resp = send(
        &app,
        request(
            "GET",
            &format!("/v1/batches/{id}/results"),
            Some(&key_b),
            None,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
