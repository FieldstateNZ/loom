//! Integration tests for the `/v1` conversation API.
//!
//! Each test boots a throwaway PostgreSQL 16 container via `testcontainers`,
//! applies the store migrations, builds the real router, and drives it through
//! `tower::ServiceExt::oneshot`. Provider behaviour is supplied either by a
//! `MockProvider` (injected through a test [`ProviderFactory`]) or, for the
//! Anthropic path, by the real `AnthropicProvider` pointed at a `wiremock`
//! server that serves a recorded SSE transcript — so no test touches a live API.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::response::Response;
use axum::Router;
use http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use loom_core::Message;
use loom_provider::mock::MockProvider;
use loom_provider::{Capability, ContentDelta, Provider, StopReason, TurnEvent, TurnEventKind};
use loom_server::{build_router, ApiError, AppState, Crypto, KeyHasher, ProviderFactory};

use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ADMIN_TOKEN: &str = "root-admin-secret-token";
const ENC_KEY: [u8; 32] = [0x22; 32];
const PEPPER: &[u8] = b"conversation-test-pepper";

/// A test [`ProviderFactory`] that always returns a fixed, pre-configured
/// provider for the `"mock"` name.
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
            Err(ApiError::bad_request("unknown provider in test"))
        }
    }
}

/// Boots a fresh migrated database and returns the container, the store, and a
/// router whose provider factory is `factory` (or the default when `None`).
async fn setup(factory: Option<Arc<dyn ProviderFactory>>) -> (ContainerAsync<Postgres>, Router) {
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

    let mut state = AppState::new(
        store,
        Crypto::new(ENC_KEY),
        KeyHasher::new(PEPPER.to_vec()),
        ADMIN_TOKEN.to_owned(),
    );
    if let Some(factory) = factory {
        state = state.with_provider_factory(factory);
    }
    (container, build_router(state))
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

/// Builds a request with a raw (possibly malformed) body and a JSON content
/// type — used to exercise the request-body extractor's error mapping.
fn raw_request(method: &str, uri: &str, bearer: Option<&str>, body: &'static str) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(http::header::CONTENT_TYPE, "application/json");
    if let Some(token) = bearer {
        builder = builder.header(http::header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::from(body)).unwrap()
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
    json_body(resp)
        .await
        .get("key")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned()
}

async fn create_conversation(app: &Router, key: &str, provider: &str, model: &str) -> Uuid {
    let resp = send(
        app,
        request(
            "POST",
            "/v1/conversations",
            Some(key),
            Some(json!({ "provider": provider, "model": model, "system": "be concise" })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    Uuid::parse_str(json_body(resp).await["id"].as_str().unwrap()).unwrap()
}

/// A canned streaming transcript for the mock provider: a single text block
/// assembled from a delta, plus a clean end-of-turn.
fn text_stream_events(text: &str) -> Vec<TurnEvent> {
    vec![
        TurnEvent::new(
            TurnEventKind::TurnStarted,
            json!({ "type": "message_start" }),
        ),
        TurnEvent::new(
            TurnEventKind::ContentPartStarted {
                index: 0,
                part: loom_core::ContentPart::text(""),
            },
            json!({ "type": "content_block_start", "index": 0 }),
        ),
        TurnEvent::new(
            TurnEventKind::ContentPartDelta {
                index: 0,
                delta: ContentDelta::Text {
                    text: text.to_owned(),
                },
            },
            json!({ "type": "content_block_delta", "index": 0 }),
        ),
        TurnEvent::new(
            TurnEventKind::ContentPartComplete {
                index: 0,
                part: loom_core::ContentPart::text(text),
            },
            json!({ "type": "content_block_stop", "index": 0 }),
        ),
        TurnEvent::new(
            TurnEventKind::TurnEnded {
                stop_reason: StopReason::EndTurn,
                usage: None,
                cost: None,
            },
            json!({ "type": "message_stop" }),
        ),
    ]
}

#[tokio::test]
async fn create_turn_and_history_with_mock() {
    let provider = MockProvider::new("mock", "mock-model", [Capability::ClientTools])
        .with_completion(Message::assistant("hi from mock"));
    let (_pg, app) = setup(Some(Arc::new(FixedFactory(Arc::new(provider))))).await;

    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key, "mock", "mock-model").await;

    // Non-streaming turn returns the assistant message.
    let turn = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(&key),
            Some(json!({ "content": [{ "type": "text", "text": "hello" }], "stream": false })),
        ),
    )
    .await;
    assert_eq!(turn.status(), StatusCode::OK);
    let body = json_body(turn).await;
    // Non-streaming turns return the `{ message, cost }` envelope; `mock`/
    // `mock-model` has no seeded price, so `cost` is `None`.
    assert_eq!(body["message"]["role"], "assistant");
    assert_eq!(body["message"]["content"][0]["text"], "hi from mock");
    assert_eq!(body["cost"], Value::Null);

    // History shows the user turn then the assistant turn, in order.
    let history = send(
        &app,
        request(
            "GET",
            &format!("/v1/conversations/{convo}"),
            Some(&key),
            None,
        ),
    )
    .await;
    assert_eq!(history.status(), StatusCode::OK);
    let convo_body = json_body(history).await;
    let messages = convo_body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"][0]["text"], "hello");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"][0]["text"], "hi from mock");
}

#[tokio::test]
async fn streaming_turn_with_mock_persists_final_message() {
    let provider = MockProvider::new("mock", "mock-model", [Capability::Streaming])
        .with_events(text_stream_events("Hi there"));
    let (_pg, app) = setup(Some(Arc::new(FixedFactory(Arc::new(provider))))).await;

    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key, "mock", "mock-model").await;

    let turn = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(&key),
            Some(json!({ "content": [{ "type": "text", "text": "hello" }], "stream": true })),
        ),
    )
    .await;
    assert_eq!(turn.status(), StatusCode::OK);
    assert_eq!(
        turn.headers()
            .get(http::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "text/event-stream"
    );
    let sse = text_body(turn).await;
    // Each SSE frame is a serialised TurnEvent (normalised kind + verbatim raw).
    assert!(sse.contains("data:"));
    assert!(sse.contains("content_part_complete"));
    assert!(sse.contains("Hi there"));
    assert!(sse.contains("\"type\":\"message_stop\"")); // verbatim native event preserved

    // The reassembled assistant turn was persisted.
    let history = send(
        &app,
        request(
            "GET",
            &format!("/v1/conversations/{convo}"),
            Some(&key),
            None,
        ),
    )
    .await;
    let messages = json_body(history).await;
    let messages = messages["messages"].as_array().unwrap().clone();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"][0]["text"], "Hi there");
}

/// The Anthropic SSE transcript served by the wiremock stand-in.
const ANTHROPIC_SSE: &str =
    include_str!("../../loom-provider-anthropic/tests/fixtures/stream_text.sse");

#[tokio::test]
async fn streaming_turn_with_anthropic_fixture() {
    // A wiremock server standing in for the Anthropic Messages API.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(ANTHROPIC_SSE),
        )
        .mount(&server)
        .await;

    // The default factory builds a real AnthropicProvider from the stored,
    // encrypted credential — including its base-URL override pointing at wiremock.
    let (_pg, app) = setup(None).await;
    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;

    let cred = send(
        &app,
        request(
            "PUT",
            &format!("/admin/tenants/{tenant}/credentials/anthropic"),
            Some(ADMIN_TOKEN),
            Some(json!({ "api_key": "sk-ant-test", "base_url": server.uri() })),
        ),
    )
    .await;
    assert_eq!(cred.status(), StatusCode::OK);

    let convo = create_conversation(&app, &key, "anthropic", "claude-opus-4-8").await;

    let turn = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(&key),
            Some(json!({ "content": [{ "type": "text", "text": "hello" }], "stream": true })),
        ),
    )
    .await;
    assert_eq!(turn.status(), StatusCode::OK);
    let sse = text_body(turn).await;
    assert!(sse.contains("turn_started"));
    assert!(sse.contains("\"type\":\"message_start\"")); // verbatim native event

    // The streamed turn reassembles to the same text the non-streaming path
    // would produce and is persisted.
    let history = send(
        &app,
        request(
            "GET",
            &format!("/v1/conversations/{convo}"),
            Some(&key),
            None,
        ),
    )
    .await;
    let body = json_body(history).await;
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"][0]["text"], "Hello, world");
}

/// A minimal non-streaming Anthropic Messages response body.
const ANTHROPIC_JSON_RESPONSE: &str = r#"{
    "id": "msg_mcp",
    "type": "message",
    "role": "assistant",
    "model": "claude-opus-4-8",
    "content": [{ "type": "text", "text": "done" }],
    "stop_reason": "end_turn",
    "usage": { "input_tokens": 5, "output_tokens": 2 }
}"#;

/// A named MCP server's authorization token is injected into the upstream
/// Anthropic request server-side, and never appears in the admin listing, the
/// turn response, or persisted history.
#[tokio::test]
async fn named_mcp_server_token_is_injected_upstream_and_never_leaks() {
    const MCP_TOKEN: &str = "mcp-secret-do-not-leak";

    // A wiremock stand-in that records the request bodies it receives.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(ANTHROPIC_JSON_RESPONSE),
        )
        .mount(&server)
        .await;

    let (_pg, app) = setup(None).await;
    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;

    // Store the Anthropic credential (base URL → wiremock).
    let cred = send(
        &app,
        request(
            "PUT",
            &format!("/admin/tenants/{tenant}/credentials/anthropic"),
            Some(ADMIN_TOKEN),
            Some(json!({ "api_key": "sk-ant-test", "base_url": server.uri() })),
        ),
    )
    .await;
    assert_eq!(cred.status(), StatusCode::OK);

    // Register the named MCP server with its secret token.
    let reg = send(
        &app,
        request(
            "PUT",
            &format!("/admin/tenants/{tenant}/mcp-servers/github"),
            Some(ADMIN_TOKEN),
            Some(json!({
                "url": "https://mcp.githubcopilot.com/mcp",
                "authorization_token": MCP_TOKEN,
                "tool_configuration": { "enabled": true }
            })),
        ),
    )
    .await;
    assert_eq!(reg.status(), StatusCode::OK);
    let reg_body = json_body(reg).await;
    // The registration response confirms a token is held but never echoes it.
    assert_eq!(reg_body["has_authorization"], json!(true));
    assert!(
        !reg_body.to_string().contains(MCP_TOKEN),
        "registration response must not echo the token"
    );

    // The admin listing likewise never exposes the token.
    let list = send(
        &app,
        request(
            "GET",
            &format!("/admin/tenants/{tenant}/mcp-servers"),
            Some(ADMIN_TOKEN),
            None,
        ),
    )
    .await;
    let list_text = text_body(list).await;
    assert!(list_text.contains("\"github\""));
    assert!(
        !list_text.contains(MCP_TOKEN),
        "admin listing must not expose the token"
    );

    // Run a turn that references the server by name only.
    let convo = create_conversation(&app, &key, "anthropic", "claude-opus-4-8").await;
    let turn = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(&key),
            Some(json!({
                "content": [{ "type": "text", "text": "search my repos" }],
                "stream": false,
                "options": { "mcp_servers": [{ "name": "github" }] }
            })),
        ),
    )
    .await;
    assert_eq!(turn.status(), StatusCode::OK);
    let turn_text = text_body(turn).await;
    assert!(
        !turn_text.contains(MCP_TOKEN),
        "the turn response must not contain the MCP token"
    );

    // The upstream request DID carry the injected token (server-side injection),
    // in the native `mcp_servers` field, with the resolved URL and config.
    let requests = server
        .received_requests()
        .await
        .expect("wiremock records requests");
    let upstream: Value =
        serde_json::from_slice(&requests.last().unwrap().body).expect("upstream body is JSON");
    let servers = upstream["mcp_servers"]
        .as_array()
        .expect("mcp_servers array");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["name"], json!("github"));
    assert_eq!(
        servers[0]["url"],
        json!("https://mcp.githubcopilot.com/mcp")
    );
    assert_eq!(servers[0]["authorization_token"], json!(MCP_TOKEN));
    assert_eq!(servers[0]["tool_configuration"], json!({ "enabled": true }));

    // Persisted history contains neither the token nor the options.
    let history = send(
        &app,
        request(
            "GET",
            &format!("/v1/conversations/{convo}"),
            Some(&key),
            None,
        ),
    )
    .await;
    let history_text = text_body(history).await;
    assert!(
        !history_text.contains(MCP_TOKEN),
        "persisted history must not contain the MCP token"
    );
}

/// A turn referencing an MCP server that is not registered for the tenant fails
/// fast with a structured `422`, without dispatching to the provider.
#[tokio::test]
async fn turn_referencing_unregistered_mcp_server_is_unprocessable() {
    let provider = MockProvider::new("mock", "mock-model", [Capability::McpConnector])
        .with_completion(Message::assistant("unused"));
    let (_pg, app) = setup(Some(Arc::new(FixedFactory(Arc::new(provider))))).await;
    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;

    let resp = send(
        &app,
        request(
            "POST",
            "/v1/turns",
            Some(&key),
            Some(json!({
                "provider": "mock",
                "model": "mock-model",
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }] }],
                "options": { "mcp_servers": [{ "name": "unregistered" }] }
            })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(resp).await;
    assert_eq!(body["error"]["code"], json!("mcp_server_not_configured"));
    let _ = tenant;
}

#[tokio::test]
async fn stateless_turn_parity_with_stateful() {
    let provider = MockProvider::new("mock", "mock-model", [Capability::ClientTools])
        .with_completion(Message::assistant("parity answer"));
    let (_pg, app) = setup(Some(Arc::new(FixedFactory(Arc::new(provider))))).await;

    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;

    // Stateless: no persistence, assistant message returned directly.
    let stateless = send(
        &app,
        request(
            "POST",
            "/v1/turns",
            Some(&key),
            Some(json!({
                "provider": "mock",
                "model": "mock-model",
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }] }],
                "stream": false
            })),
        ),
    )
    .await;
    assert_eq!(stateless.status(), StatusCode::OK);
    let stateless_body = json_body(stateless).await;

    // Stateful path over the same input.
    let convo = create_conversation(&app, &key, "mock", "mock-model").await;
    let stateful = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(&key),
            Some(json!({ "content": [{ "type": "text", "text": "hi" }], "stream": false })),
        ),
    )
    .await;
    let stateful_body = json_body(stateful).await;

    // Same assistant message — and the same (unpriced) `cost` — from both
    // paths: both endpoints share the `{ message, cost }` envelope.
    assert_eq!(stateless_body["message"]["role"], "assistant");
    assert_eq!(
        stateless_body["message"]["content"],
        stateful_body["message"]["content"]
    );
    assert_eq!(
        stateless_body["message"]["content"][0]["text"],
        "parity answer"
    );
    assert_eq!(stateless_body["cost"], stateful_body["cost"]);
}

/// Collects every local `$ref` string found anywhere in a JSON document.
fn collect_schema_refs(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                if k == "$ref" {
                    if let Some(s) = val.as_str() {
                        out.push(s.to_owned());
                    }
                } else {
                    collect_schema_refs(val, out);
                }
            }
        }
        serde_json::Value::Array(arr) => arr.iter().for_each(|e| collect_schema_refs(e, out)),
        _ => {}
    }
}

#[tokio::test]
async fn openapi_document_is_valid() {
    let (_pg, app) = setup(None).await;
    let resp = send(&app, request("GET", "/openapi.json", None, None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let doc = json_body(resp).await;

    // Deserialises as a typed OpenAPI document, and is a 3.x spec exposing our
    // conversation routes.
    // A 3.x document that is self-consistent: every local `$ref` resolves to a
    // defined component (no dangling references). We deliberately do NOT
    // round-trip through utoipa's own `OpenApi` deserialiser: utoipa 5 emits
    // valid OpenAPI 3.1 "any" schemas for arbitrary-JSON fields (e.g.
    // `Usage::raw`) that its *own* Deserialize cannot read back — a utoipa
    // limitation, not a defect in the served document, which `openapi-typescript`
    // consumes without issue.
    assert!(doc["openapi"].as_str().unwrap().starts_with("3."));
    let schemas = doc["components"]["schemas"]
        .as_object()
        .expect("component schemas present");
    let mut refs = Vec::new();
    collect_schema_refs(&doc, &mut refs);
    assert!(
        !refs.is_empty(),
        "document should reference component schemas"
    );
    for r in &refs {
        if let Some(name) = r.strip_prefix("#/components/schemas/") {
            assert!(schemas.contains_key(name), "dangling $ref: {r}");
        }
    }
    let paths = doc["paths"].as_object().unwrap();
    assert!(paths.contains_key("/v1/conversations"));
    assert!(paths.contains_key("/v1/conversations/{id}/turns"));
    assert!(paths.contains_key("/v1/turns"));
    // The full tenant surface, including whoami, is documented.
    assert!(paths.contains_key("/v1/whoami"));

    // The `virtual_key` security scheme referenced by the guarded operations is
    // defined in components — no dangling reference.
    let schemes = doc["components"]["securitySchemes"]
        .as_object()
        .expect("security schemes present");
    assert!(schemes.contains_key("virtual_key"));
    assert_eq!(
        doc["components"]["securitySchemes"]["virtual_key"]["type"],
        "http"
    );
    assert_eq!(
        doc["components"]["securitySchemes"]["virtual_key"]["scheme"],
        "bearer"
    );
}

#[tokio::test]
async fn turn_on_foreign_conversation_is_not_found() {
    let provider = MockProvider::new("mock", "mock-model", [Capability::ClientTools])
        .with_completion(Message::assistant("secret"));
    let (_pg, app) = setup(Some(Arc::new(FixedFactory(Arc::new(provider))))).await;

    let tenant_a = create_tenant(&app, "tenant-a").await;
    let tenant_b = create_tenant(&app, "tenant-b").await;
    let key_a = create_key(&app, tenant_a).await;
    let key_b = create_key(&app, tenant_b).await;

    let convo = create_conversation(&app, &key_a, "mock", "mock-model").await;

    // Tenant B may not append a turn to tenant A's conversation.
    let resp = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(&key_b),
            Some(json!({ "content": [{ "type": "text", "text": "intrude" }] })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(resp).await["error"]["code"], "not_found");
}

#[tokio::test]
async fn capability_unsupported_returns_422() {
    // The model declares only client tools; a vision request must be rejected.
    let provider = MockProvider::new("mock", "mock-model", [Capability::ClientTools]);
    let (_pg, app) = setup(Some(Arc::new(FixedFactory(Arc::new(provider))))).await;

    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key, "mock", "mock-model").await;

    let resp = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(&key),
            Some(json!({
                "content": [
                    { "type": "image", "source": { "type": "url", "url": "https://example.com/a.png" } }
                ]
            })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(resp).await;
    assert_eq!(body["error"]["code"], "capability_unsupported");
    assert_eq!(body["error"]["provider_error"]["capability"], "vision");

    // The doomed user turn was not persisted.
    let history = send(
        &app,
        request(
            "GET",
            &format!("/v1/conversations/{convo}"),
            Some(&key),
            None,
        ),
    )
    .await;
    assert_eq!(
        json_body(history).await["messages"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn provider_not_configured_returns_422() {
    // Default factory, but no anthropic credential stored for this tenant.
    let (_pg, app) = setup(None).await;
    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;
    let convo = create_conversation(&app, &key, "anthropic", "claude-opus-4-8").await;

    let resp = send(
        &app,
        request(
            "POST",
            &format!("/v1/conversations/{convo}/turns"),
            Some(&key),
            Some(json!({ "content": [{ "type": "text", "text": "hello" }] })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        json_body(resp).await["error"]["code"],
        "provider_not_configured"
    );
}

#[tokio::test]
async fn malformed_json_body_returns_enveloped_400() {
    let (_pg, app) = setup(None).await;
    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;

    // A truncated body is not valid JSON: the custom extractor renders it as a
    // 400 with the shared error envelope, not axum's default text/plain body.
    let resp = send(
        &app,
        raw_request("POST", "/v1/conversations", Some(&key), "{"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let content_type = resp
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        content_type.starts_with("application/json"),
        "expected an enveloped JSON body, got content-type {content_type:?}"
    );
    let body = json_body(resp).await;
    assert_eq!(body["error"]["code"], "bad_request");
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn wrong_shape_json_body_returns_enveloped_422() {
    let (_pg, app) = setup(None).await;
    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;

    // Well-formed JSON but the wrong shape (the required `provider` is missing):
    // a 422 rendered through the shared envelope.
    let resp = send(
        &app,
        request(
            "POST",
            "/v1/conversations",
            Some(&key),
            Some(json!({ "model": "mock-model" })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(resp).await;
    assert_eq!(body["error"]["code"], "unprocessable_entity");
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn cross_tenant_delete_is_denied() {
    let (_pg, app) = setup(None).await;
    let tenant_a = create_tenant(&app, "tenant-a").await;
    let tenant_b = create_tenant(&app, "tenant-b").await;
    let key_a = create_key(&app, tenant_a).await;
    let key_b = create_key(&app, tenant_b).await;

    // A conversation owned by tenant A.
    let convo = create_conversation(&app, &key_a, "mock", "mock-model").await;

    // Tenant B may not delete tenant A's conversation: the store scopes the
    // delete, yielding a 404 rather than a silent cross-tenant deletion.
    let del = send(
        &app,
        request(
            "DELETE",
            &format!("/v1/conversations/{convo}"),
            Some(&key_b),
            None,
        ),
    )
    .await;
    assert_eq!(del.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(del).await["error"]["code"], "not_found");

    // Tenant A's conversation still exists.
    let get = send(
        &app,
        request(
            "GET",
            &format!("/v1/conversations/{convo}"),
            Some(&key_a),
            None,
        ),
    )
    .await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(json_body(get).await["id"], convo.to_string());
}

#[tokio::test]
async fn stateless_turn_rejects_empty_messages() {
    let (_pg, app) = setup(None).await;
    let tenant = create_tenant(&app, "acme").await;
    let key = create_key(&app, tenant).await;

    // Parity with `create_turn`'s empty-content check: an empty `messages`
    // array is a 400 with the shared envelope, before any provider is resolved.
    let resp = send(
        &app,
        request(
            "POST",
            "/v1/turns",
            Some(&key),
            Some(json!({
                "provider": "mock",
                "model": "mock-model",
                "messages": []
            })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"]["code"], "bad_request");
}
