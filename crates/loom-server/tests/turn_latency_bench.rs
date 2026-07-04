//! Turn latency benchmark (issue #25): stateful vs. stateless turn latency,
//! isolating *Loom's own* overhead from model latency.
//!
//! Like `tests/conversations.rs`, this boots a throwaway PostgreSQL 16
//! container via `testcontainers`, builds the real router, and drives it
//! through `tower::ServiceExt::oneshot`. The provider is the in-repo
//! `MockProvider` — deterministic and in-memory, with no network call — so the
//! measured wall time reflects Loom's HTTP/auth/store/attribution path only,
//! not a model's think time.
//!
//! This is a **timing benchmark**, not a correctness check, so it is
//! `#[ignore]`d rather than run as part of the default `cargo test` suite (it
//! also needs a Docker daemon for `testcontainers`, which is not available in
//! every environment this repo is built in). Run it explicitly where Docker is
//! available (CI or a Docker-capable host):
//!
//! ```text
//! cargo test -p loom-server --test turn_latency_bench -- --ignored --nocapture
//! ```
//!
//! See `docs/benchmarks/turn-latency.md` for methodology, the full per-path
//! round-trip enumeration this file's assertions mirror, and the resulting
//! do-now/defer decision.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use axum::response::Response;
use axum::Router;
use http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use uuid::Uuid;

use loom_core::Message;
use loom_provider::mock::MockProvider;
use loom_provider::{Capability, Provider};
use loom_server::{build_router, ApiError, AppState, Crypto, KeyHasher, ProviderFactory};

use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

const ADMIN_TOKEN: &str = "root-admin-secret-token";
const ENC_KEY: [u8; 32] = [0x77; 32];
const PEPPER: &[u8] = b"turn-latency-bench-pepper";

/// Timed iterations per path, after warmup. Modest on purpose: the point is a
/// stable median/p95 from one real Postgres container, not a statistically
/// bulletproof distribution.
const ITERATIONS: usize = 50;

/// Warmup requests run (and discarded) before timing starts, per path — clears
/// pool connection setup and any first-call effects out of the timed samples.
const WARMUP: usize = 3;

/// Counts `tracing` events sqlx-core emits at `target: "sqlx::query"` — one
/// per *logged* statement (see `sqlx-core`'s `QueryLogger::finish`, which fires
/// at `tracing_level` = `Debug` by default for every `execute`/`fetch_*`).
///
/// This under-counts the true wire round-trip count by one per store-level
/// transaction: a transaction's `BEGIN` is sent via
/// `PgTransactionManager::begin` using the Postgres simple-query protocol
/// directly (`queue_simple_query` + `wait_until_ready`) and never passes
/// through the logged `Executor::execute` path that ordinary queries and
/// `COMMIT` use (`PgTransactionManager::commit` *does* call `conn.execute(..)`,
/// so `COMMIT` **is** logged). Concretely: `ConversationStore::append_message`
/// (`crates/loom-store/src/pg/conversation.rs`) is one transaction — `BEGIN`,
/// `SELECT … FOR UPDATE`, `INSERT … RETURNING seq`, `UPDATE`, `COMMIT` — five
/// wire round trips, but only four show up here.
///
/// It is still a precise, deterministic proxy for *relative* query counts
/// between the two paths, and a regression guard against a future change
/// silently adding or removing a round trip. See
/// `docs/benchmarks/turn-latency.md` for the full accounting (true round
/// trips vs. logged events).
#[derive(Clone, Default)]
struct QueryCounter(Arc<AtomicUsize>);

impl QueryCounter {
    fn snapshot(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }

    fn reset(&self) {
        self.0.store(0, Ordering::Relaxed);
    }
}

impl<S: tracing::Subscriber> Layer<S> for QueryCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() == "sqlx::query" {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A test [`ProviderFactory`] that always returns a fixed, pre-configured
/// provider for the `"mock"` name (mirrors `tests/conversations.rs`).
struct FixedFactory(Arc<dyn Provider>);

#[async_trait]
impl ProviderFactory for FixedFactory {
    async fn provider(
        &self,
        _state: &AppState,
        _tenant_id: Uuid,
        provider: &str,
    ) -> Result<Arc<dyn Provider>, ApiError> {
        if provider == "mock" {
            Ok(self.0.clone())
        } else {
            Err(ApiError::bad_request("unknown provider in bench"))
        }
    }
}

/// Boots a fresh migrated database, a router bound to a `MockProvider`, and
/// installs the process-global [`QueryCounter`] layer.
///
/// Installing a global `tracing` subscriber is a once-per-process operation
/// (as in `tests/telemetry.rs`); this file has exactly one test, so it runs
/// exactly once.
async fn setup() -> (ContainerAsync<Postgres>, Router, QueryCounter) {
    let counter = QueryCounter::default();
    tracing_subscriber::registry().with(counter.clone()).init();

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
    let store = loom_store::PgStore::connect(&url)
        .await
        .expect("connect to postgres");
    loom_store::run_migrations(store.pool())
        .await
        .expect("migrations apply cleanly");

    let provider = MockProvider::new("mock", "mock-model", [Capability::ClientTools])
        .with_completion(Message::assistant("mock reply"));
    let state = AppState::new(
        store,
        Crypto::new(ENC_KEY),
        KeyHasher::new(PEPPER.to_vec()),
        ADMIN_TOKEN.to_owned(),
    )
    .with_provider_factory(Arc::new(FixedFactory(Arc::new(provider))));
    (container, build_router(state), counter)
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
            Some(json!({ "name": "bench" })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    json_body(resp).await["key"].as_str().unwrap().to_owned()
}

async fn create_conversation(app: &Router, key: &str) -> Uuid {
    let resp = send(
        app,
        request(
            "POST",
            "/v1/conversations",
            Some(key),
            Some(json!({ "provider": "mock", "model": "mock-model" })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    Uuid::parse_str(json_body(resp).await["id"].as_str().unwrap()).unwrap()
}

/// A `POST /v1/conversations/{id}/turns` request (non-streaming).
fn stateful_turn_request(convo: Uuid, key: &str) -> Request<Body> {
    request(
        "POST",
        &format!("/v1/conversations/{convo}/turns"),
        Some(key),
        Some(json!({
            "content": [{ "type": "text", "text": "hello" }],
            "stream": false,
        })),
    )
}

/// A `POST /v1/turns` request (non-streaming): the whole conversation is
/// supplied inline, nothing is persisted.
fn stateless_turn_request(key: &str) -> Request<Body> {
    request(
        "POST",
        "/v1/turns",
        Some(key),
        Some(json!({
            "provider": "mock",
            "model": "mock-model",
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "hello" }] },
            ],
            "stream": false,
        })),
    )
}

/// Times `iters` sequential runs of `make_request` (freshly built each time,
/// since a `Request<Body>` is consumed by `send`), returning the per-iteration
/// wall time and the [`QueryCounter`] snapshot taken right after each request.
async fn timed_runs(
    app: &Router,
    counter: &QueryCounter,
    iters: usize,
    mut make_request: impl FnMut() -> Request<Body>,
) -> (Vec<Duration>, Vec<usize>) {
    let mut times = Vec::with_capacity(iters);
    let mut queries = Vec::with_capacity(iters);
    for _ in 0..iters {
        counter.reset();
        let started = Instant::now();
        let resp = send(app, make_request()).await;
        times.push(started.elapsed());
        assert_eq!(resp.status(), StatusCode::OK);
        queries.push(counter.snapshot());
    }
    (times, queries)
}

/// The median and p95 of `times` (nearest-rank; `times` is sorted in place).
fn median_p95(times: &mut [Duration]) -> (Duration, Duration) {
    times.sort_unstable();
    let median = times[times.len() / 2];
    #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
    let p95_rank = ((times.len() as f64) * 0.95).ceil() as usize;
    let p95 = times[p95_rank.saturating_sub(1).min(times.len() - 1)];
    (median, p95)
}

fn as_ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[tokio::test]
#[ignore = "needs a Docker daemon for testcontainers Postgres; run with `-- --ignored`"]
async fn stateful_vs_stateless_turn_latency() {
    let (_pg, app, queries) = setup().await;

    let tenant = create_tenant(&app, "bench").await;
    let key = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key).await;

    for _ in 0..WARMUP {
        send(&app, stateful_turn_request(convo, &key)).await;
        send(&app, stateless_turn_request(&key)).await;
    }

    let (mut stateful_times, stateful_queries) = timed_runs(&app, &queries, ITERATIONS, || {
        stateful_turn_request(convo, &key)
    })
    .await;
    let (mut stateless_times, stateless_queries) =
        timed_runs(&app, &queries, ITERATIONS, || stateless_turn_request(&key)).await;

    let (stateful_median, stateful_p95) = median_p95(&mut stateful_times);
    let (stateless_median, stateless_p95) = median_p95(&mut stateless_times);

    println!(
        "stateful  turn (N={ITERATIONS}): median={:.2}ms p95={:.2}ms  logged sqlx::query events/turn={}",
        as_ms(stateful_median),
        as_ms(stateful_p95),
        stateful_queries[0],
    );
    println!(
        "stateless turn (N={ITERATIONS}): median={:.2}ms p95={:.2}ms  logged sqlx::query events/turn={}",
        as_ms(stateless_median),
        as_ms(stateless_p95),
        stateless_queries[0],
    );

    // Deterministic regression guard, not a timing assertion: with no budget
    // configured on the tenant or key, `budget::enforce`'s spend query never
    // runs (see docs/benchmarks/turn-latency.md), so the logged-query count is
    // constant across iterations and mirrors the code-grounded round-trip
    // enumeration exactly (modulo the un-logged `BEGIN` explained on
    // `QueryCounter`). A change to these counts is exactly the kind of
    // round-trip-count drift this benchmark exists to catch.
    assert!(
        stateful_queries.iter().all(|&n| n == stateful_queries[0]),
        "stateful per-turn logged query count should be constant, got {stateful_queries:?}"
    );
    assert!(
        stateless_queries.iter().all(|&n| n == stateless_queries[0]),
        "stateless per-turn logged query count should be constant, got {stateless_queries:?}"
    );
    assert_eq!(
        stateful_queries[0], 16,
        "stateful turn: logged sqlx::query events (see docs/benchmarks/turn-latency.md)"
    );
    assert_eq!(
        stateless_queries[0], 6,
        "stateless turn: logged sqlx::query events (see docs/benchmarks/turn-latency.md)"
    );
    assert!(stateless_queries[0] < stateful_queries[0]);
}
