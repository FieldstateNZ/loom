//! Integration test for the mid-stream client-disconnect path of a streaming
//! turn.
//!
//! A streaming turn increments the `loom.streams.active` gauge when the SSE
//! response is built and is expected to decrement it again when the stream ends.
//! Before the RAII fix, the decrement lived only in the unfold's clean-end/error
//! branches, so a client that disconnected mid-stream (hyper drops the SSE body
//! future) leaked the gauge forever and never recorded the partial turn.
//!
//! This test drives a streaming turn through the full router with a
//! `MockProvider`, reads a single SSE frame, then **drops the body before the
//! terminal event** — simulating a disconnect — and asserts:
//!
//! - `loom.streams.active` returns to its pre-stream value (`0`); the RAII guard
//!   balanced the gauge on the drop path.
//! - a usage event was still recorded for the dropped turn (the best-effort
//!   settlement spawned from `SseState`'s `Drop`), allowing a bounded poll for
//!   the detached task to complete.
//!
//! It installs an in-memory metric exporter as the process-global meter provider
//! (its own test binary, so this does not collide with the span-exporter test in
//! `telemetry.rs`).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::response::Response;
use axum::Router;
use futures::StreamExt;
use http::{Request, StatusCode};
use serde_json::{json, Value as JsonValue};
use tower::ServiceExt;
use uuid::Uuid;

use loom_core::Usage;
use loom_provider::mock::MockProvider;
use loom_provider::{Capability, ContentDelta, Provider, StopReason, TurnEvent, TurnEventKind};
use loom_server::{build_router, ApiError, AppState, Crypto, KeyHasher, ProviderFactory};

use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

const ADMIN_TOKEN: &str = "root-admin-secret-token";
const ENC_KEY: [u8; 32] = [0x51; 32];
const PEPPER: &[u8] = b"stream-disconnect-test-pepper";

/// A factory returning a fixed provider for any provider name.
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

async fn setup(provider: Arc<dyn Provider>) -> (ContainerAsync<Postgres>, Router) {
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

    let state = AppState::new(
        store,
        Crypto::new(ENC_KEY),
        KeyHasher::new(PEPPER.to_vec()),
        ADMIN_TOKEN.to_owned(),
    )
    .with_provider_factory(Arc::new(AnyFactory(provider)));
    (container, build_router(state))
}

async fn send(app: &Router, req: Request<Body>) -> Response {
    app.clone().oneshot(req).await.expect("router handled")
}

async fn json_body(resp: Response) -> JsonValue {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("body is JSON")
}

fn request(
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<JsonValue>,
) -> Request<Body> {
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

async fn create_conversation(app: &Router, key: &str, provider: &str, model: &str) -> Uuid {
    let resp = send(
        app,
        request(
            "POST",
            "/v1/conversations",
            Some(key),
            Some(json!({ "provider": provider, "model": model })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    Uuid::parse_str(json_body(resp).await["id"].as_str().unwrap()).unwrap()
}

/// A multi-event streaming transcript: start, a text delta, a completed content
/// part, then a clean end-of-turn carrying the usage snapshot. The disconnect is
/// injected after the first frame, so the terminal event is never delivered.
fn stream_events() -> Vec<TurnEvent> {
    let mut usage = Usage::new();
    usage.input_tokens = Some(12);
    usage.output_tokens = Some(7);
    vec![
        TurnEvent::new(
            TurnEventKind::TurnStarted,
            json!({ "type": "message_start" }),
        ),
        TurnEvent::new(
            TurnEventKind::ContentPartDelta {
                index: 0,
                delta: ContentDelta::Text {
                    text: "partial".to_owned(),
                },
            },
            json!({ "type": "content_block_delta", "index": 0 }),
        ),
        TurnEvent::new(
            TurnEventKind::ContentPartComplete {
                index: 0,
                part: loom_core::ContentPart::text("partial"),
            },
            json!({ "type": "content_block_stop", "index": 0 }),
        ),
        TurnEvent::new(
            TurnEventKind::TurnEnded {
                stop_reason: StopReason::EndTurn,
                usage: Some(usage),
            },
            json!({ "type": "message_stop" }),
        ),
    ]
}

/// The net value of the named `i64` up/down-counter across all exported data
/// points, or `None` if the instrument was not exported.
fn gauge_value(metrics: &[ResourceMetrics], name: &str) -> Option<i64> {
    for rm in metrics {
        for scope in rm.scope_metrics() {
            for m in scope.metrics() {
                if m.name() == name {
                    if let AggregatedMetrics::I64(MetricData::Sum(sum)) = m.data() {
                        return Some(sum.data_points().map(|dp| dp.value()).sum());
                    }
                }
            }
        }
    }
    None
}

/// Total usage events recorded for `tenant`, read back through the public usage
/// rollup endpoint.
async fn usage_event_count(app: &Router, key: &str) -> i64 {
    let resp = send(
        app,
        request("GET", "/v1/usage?group_by=model", Some(key), None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    json_body(resp).await["rows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| r["event_count"].as_i64().unwrap_or(0))
                .sum()
        })
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_disconnect_balances_gauge_and_records_usage() {
    // Install an in-memory metric exporter as the process-global meter provider,
    // *before* building any `AppState`, so `Metrics::new()` (via the global meter)
    // routes the active-streams gauge to this reader.
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone()).build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    opentelemetry::global::set_meter_provider(provider.clone());

    let stream_provider =
        MockProvider::new("anthropic", "claude-opus-4-8", [Capability::Streaming])
            .with_events(stream_events());
    let (_pg, app) = setup(Arc::new(stream_provider)).await;
    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key, "anthropic", "claude-opus-4-8").await;

    // Begin the streaming turn. Building the SSE response increments the gauge.
    let resp = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(&key),
            Some(json!({
                "content": [{ "type": "text", "text": "hello" }],
                "stream": true
            })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Read exactly one SSE frame to prove the stream started emitting, then drop
    // the body *before* the terminal event — the mid-stream disconnect. Dropping
    // the body drops the unfold future (and `SseState`) without `finalize` ever
    // running.
    {
        let mut body = resp.into_body().into_data_stream();
        let first = body.next().await;
        assert!(
            matches!(first, Some(Ok(_))),
            "stream should emit at least one frame before the disconnect"
        );
        // `body` dropped here, mid-stream.
    }

    // The best-effort settlement runs on a detached task; poll the usage rollup
    // until it lands (bounded, so a genuine regression still fails the test).
    let mut recorded = 0;
    for _ in 0..50 {
        recorded = usage_event_count(&app, &key).await;
        if recorded >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        recorded >= 1,
        "a usage event should be recorded for the disconnected stream, got {recorded}"
    );

    // The active-streams gauge must be back to its pre-stream value: the RAII
    // guard decremented it on the drop path. A leak would leave it at +1.
    provider.force_flush().expect("flush metrics");
    let exported = exporter.get_finished_metrics().expect("collect metrics");
    let gauge =
        gauge_value(&exported, "loom.streams.active").expect("loom.streams.active gauge exported");
    assert_eq!(
        gauge, 0,
        "active-streams gauge leaked after client disconnect: {gauge}"
    );
}
