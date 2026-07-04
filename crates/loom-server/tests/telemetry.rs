//! Integration tests for OpenTelemetry tracing.
//!
//! These install an in-memory [`InMemorySpanExporter`] behind the real
//! `tracing-opentelemetry` bridge (no collector needed), drive turns through the
//! full router with a `MockProvider`, and assert on the exported spans:
//!
//! - a `provider.call` span carries the model, token counts, computed cost, stop
//!   reason and tenant id, nested under the turn and HTTP request spans;
//! - **no** prompt or completion content appears in any exported span by
//!   default, and it appears only when the debug capture flag is set;
//! - the `x-request-id` echoed on the response matches the request span.
//!
//! The whole file is a single test because installing a global tracing
//! subscriber is a once-per-process operation; the phases share one exporter and
//! reset it between them.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::response::Response;
use axum::Router;
use http::{Request, StatusCode};
use serde_json::{json, Value as JsonValue};
use tower::ServiceExt;
use uuid::Uuid;

use loom_core::{Message, Usage};
use loom_provider::mock::MockProvider;
use loom_provider::{Capability, ContentDelta, Provider, StopReason, TurnEvent, TurnEventKind};
use loom_server::{build_router, ApiError, AppState, Crypto, KeyHasher, ProviderFactory};

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const ADMIN_TOKEN: &str = "root-admin-secret-token";
const ENC_KEY: [u8; 32] = [0x51; 32];
const PEPPER: &[u8] = b"telemetry-test-pepper";

/// A distinctive user-prompt string that must never appear in telemetry by
/// default.
const PROMPT_CANARY: &str = "supersecret-prompt-canary-42";
/// A distinctive assistant-completion string that must never appear in
/// telemetry by default.
const REPLY_CANARY: &str = "supersecret-reply-canary-99";

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

async fn text_body(resp: Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    String::from_utf8(bytes.to_vec()).expect("body is UTF-8")
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

/// A mock non-streaming completion reporting usage and a stop reason, so the
/// provider span has token/cost/stop-reason attributes to record.
fn completion_with_usage() -> Message {
    let mut usage = Usage::new();
    usage.input_tokens = Some(100);
    usage.output_tokens = Some(40);
    let mut message = Message::assistant(REPLY_CANARY);
    message.usage = Some(usage);
    message.raw = Some(json!({ "stop_reason": "end_turn" }));
    message
}

/// A streaming transcript: a single text block with a delta, a final usage
/// snapshot, and a clean end-of-turn carrying the stop reason.
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
                    text: REPLY_CANARY.to_owned(),
                },
            },
            json!({ "type": "content_block_delta", "index": 0 }),
        ),
        TurnEvent::new(
            TurnEventKind::ContentPartComplete {
                index: 0,
                part: loom_core::ContentPart::text(REPLY_CANARY),
            },
            json!({ "type": "content_block_stop", "index": 0 }),
        ),
        TurnEvent::new(
            TurnEventKind::TurnEnded {
                stop_reason: StopReason::EndTurn,
                usage: Some(usage),
                cost: None,
            },
            json!({ "type": "message_stop" }),
        ),
    ]
}

/// Flattens every exported span's attributes into `(key, value)` string pairs.
fn attr_pairs(spans: &[opentelemetry_sdk::trace::SpanData]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for span in spans {
        for kv in &span.attributes {
            out.push((kv.key.as_str().to_owned(), kv.value.as_str().into_owned()));
        }
    }
    out
}

/// The span with the given name, if present.
fn span_named<'a>(
    spans: &'a [opentelemetry_sdk::trace::SpanData],
    name: &str,
) -> Option<&'a opentelemetry_sdk::trace::SpanData> {
    spans.iter().find(|s| s.name == name)
}

/// Asserts no exported span, in attributes or events, contains `needle`.
fn assert_absent(spans: &[opentelemetry_sdk::trace::SpanData], needle: &str) {
    let dump = format!("{spans:?}");
    assert!(
        !dump.contains(needle),
        "telemetry unexpectedly contained {needle:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn traces_carry_usage_and_never_leak_content() {
    // Install an in-memory span exporter behind the real tracing-opentelemetry
    // bridge as the global subscriber (once per process).
    let exporter = InMemorySpanExporter::default();
    let tracer_provider = SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
        .build();
    let tracer = tracer_provider.tracer("loom-test");
    tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .init();

    // ---- Phase A: content capture OFF (the privacy default) ----
    loom_server::telemetry::set_capture_content(false);
    exporter.reset();

    let provider = MockProvider::new("anthropic", "claude-opus-4-8", [Capability::ClientTools])
        .with_completion(completion_with_usage());
    let (_pg, app) = setup(Arc::new(provider)).await;
    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key, "anthropic", "claude-opus-4-8").await;

    let resp = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(&key),
            Some(json!({
                "content": [{ "type": "text", "text": PROMPT_CANARY }],
                "stream": false
            })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let request_id = resp
        .headers()
        .get("x-request-id")
        .expect("x-request-id echoed")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(!request_id.is_empty());
    let _ = json_body(resp).await;

    tracer_provider.force_flush().expect("flush spans");
    let spans = exporter.get_finished_spans().expect("collect spans");

    // A provider.call span exists, nested under the turn and HTTP spans. The
    // HTTP span's exported name is the low-cardinality route (via `otel.name`),
    // so it is located by the request id it carries rather than by name.
    let provider_span = span_named(&spans, "provider.call").expect("provider.call span exists");
    let turn_span = span_named(&spans, "conversation.turn").expect("conversation.turn span exists");
    let http_span = spans
        .iter()
        .find(|s| {
            s.attributes.iter().any(|kv| {
                kv.key.as_str() == "loom.request_id" && kv.value.as_str().into_owned() == request_id
            })
        })
        .expect("http request span for the turn exists");
    assert_eq!(
        provider_span.parent_span_id,
        turn_span.span_context.span_id(),
        "provider.call must nest under conversation.turn"
    );
    assert_eq!(
        turn_span.parent_span_id,
        http_span.span_context.span_id(),
        "conversation.turn must nest under the http request span"
    );

    // The provider span carries model, tokens, cost, stop reason and tenant.
    let pairs = attr_pairs(std::slice::from_ref(provider_span));
    let has = |k: &str, v: &str| pairs.iter().any(|(pk, pv)| pk == k && pv == v);
    let has_key = |k: &str| pairs.iter().any(|(pk, _)| pk == k);
    assert!(has("gen_ai.request.model", "claude-opus-4-8"), "{pairs:?}");
    assert!(has("gen_ai.usage.input_tokens", "100"), "{pairs:?}");
    assert!(has("gen_ai.usage.output_tokens", "40"), "{pairs:?}");
    assert!(
        has("gen_ai.response.finish_reason", "end_turn"),
        "{pairs:?}"
    );
    assert!(has("loom.tenant.id", &tenant.to_string()), "{pairs:?}");
    assert!(has_key("loom.cost_usd"), "cost missing: {pairs:?}");

    // The HTTP span carries the request id echoed on the response.
    let http_pairs = attr_pairs(std::slice::from_ref(http_span));
    assert!(
        http_pairs
            .iter()
            .any(|(k, v)| k == "loom.request_id" && v == &request_id),
        "request id {request_id} not on http span: {http_pairs:?}"
    );

    // No prompt or completion content anywhere in the exported spans.
    assert_absent(&spans, PROMPT_CANARY);
    assert_absent(&spans, REPLY_CANARY);

    // ---- Phase B: content capture ON (opt-in debug flag) ----
    loom_server::telemetry::set_capture_content(true);
    exporter.reset();

    let convo_b = create_conversation(&app, &key, "anthropic", "claude-opus-4-8").await;
    let resp = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo_b}/turns"),
            Some(&key),
            Some(json!({
                "content": [{ "type": "text", "text": PROMPT_CANARY }],
                "stream": false
            })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let _ = json_body(resp).await;
    tracer_provider.force_flush().expect("flush spans");
    let spans = exporter.get_finished_spans().expect("collect spans");
    let dump = format!("{spans:?}");
    assert!(
        dump.contains(PROMPT_CANARY),
        "capture flag set but prompt content absent"
    );
    assert!(
        dump.contains(REPLY_CANARY),
        "capture flag set but completion content absent"
    );

    // ---- Phase C: streaming keeps the span open and records first-token ----
    loom_server::telemetry::set_capture_content(false);
    exporter.reset();

    let stream_provider =
        MockProvider::new("anthropic", "claude-opus-4-8", [Capability::Streaming])
            .with_events(stream_events());
    let (_pg2, stream_app) = setup(Arc::new(stream_provider)).await;
    let tenant2 = create_tenant(&stream_app, "beta").await;
    let key2 = create_key(&stream_app, tenant2).await;
    let convo2 = create_conversation(&stream_app, &key2, "anthropic", "claude-opus-4-8").await;

    let resp = send(
        &stream_app,
        request(
            "POST",
            &format!("/v1/conversations/{convo2}/turns"),
            Some(&key2),
            Some(json!({
                "content": [{ "type": "text", "text": PROMPT_CANARY }],
                "stream": true
            })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    // Draining the SSE body drives the stream to completion, closing the span.
    let sse = text_body(resp).await;
    assert!(sse.contains("data:"));

    tracer_provider.force_flush().expect("flush spans");
    let spans = exporter.get_finished_spans().expect("collect spans");
    let provider_span = span_named(&spans, "provider.call").expect("streaming provider.call span");
    let pairs = attr_pairs(std::slice::from_ref(provider_span));
    assert!(
        pairs.iter().any(|(k, v)| k == "loom.stream" && v == "true"),
        "stream flag missing: {pairs:?}"
    );
    assert!(
        pairs.iter().any(|(k, _)| k == "loom.first_token_ms"),
        "first-token latency missing: {pairs:?}"
    );
    assert!(
        pairs
            .iter()
            .any(|(k, v)| k == "gen_ai.usage.output_tokens" && v == "7"),
        "streamed usage missing: {pairs:?}"
    );
    // The first-token span event was recorded (its message rides in the event's
    // name or attributes depending on the bridge, so match the whole dump).
    let events_dump = format!("{:?}", provider_span.events);
    assert!(
        events_dump.contains("first token"),
        "first-token span event missing: {events_dump}"
    );
    // Content still absent on the default path.
    assert_absent(&spans, PROMPT_CANARY);
    assert_absent(&spans, REPLY_CANARY);
}
