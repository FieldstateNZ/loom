//! Real router, encrypted provider credentials and PostgreSQL persistence.
//! The only simulated service is OpenAI's Responses endpoint.

use axum::body::Body;
use axum::Router;
use http::{Request, StatusCode};
use loom_server::{build_router, AppState, Crypto, KeyHasher};
use serde_json::{json, Value};
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ADMIN: &str = "synthetic-openai-root-token";
async fn call(
    app: &Router,
    method: &str,
    url: &str,
    key: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder()
        .method(method)
        .uri(url)
        .header("authorization", format!("Bearer {key}"));
    let body = if let Some(body) = body {
        request = request.header("content-type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    (
        response.status(),
        axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
}
fn parsed(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}
async fn identity(app: &Router, slug: &str) -> (String, String) {
    let (status, bytes) = call(
        app,
        "POST",
        "/admin/tenants",
        ADMIN,
        Some(json!({"slug":slug,"name":slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let tenant = parsed(&bytes)["id"].as_str().unwrap().to_owned();
    let (status, bytes) = call(
        app,
        "POST",
        &format!("/admin/tenants/{tenant}/keys"),
        ADMIN,
        Some(json!({"name":"synthetic","env":"test"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    (tenant, parsed(&bytes)["key"].as_str().unwrap().to_owned())
}

#[tokio::test]
async fn openai_stream_is_persisted_replayed_tenant_scoped_and_capability_checked() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_bytes!("../../loom-provider-openai/tests/fixtures/stream-text.sse").as_slice(),
            "text/event-stream",
        ))
        .expect(2)
        .mount(&server)
        .await;
    let pg = Postgres::default().with_tag("16").start().await.unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let store = loom_store::PgStore::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
    ))
    .await
    .unwrap();
    loom_store::run_migrations(store.pool()).await.unwrap();
    let state = AppState::new(
        store,
        Crypto::new([0x48; 32]),
        KeyHasher::new(b"synthetic-openai-pepper".to_vec()),
        ADMIN.into(),
    );
    let app = build_router(state);
    let (alice, key) = identity(&app, "openai-alice").await;
    let (_, bob) = identity(&app, "openai-bob").await;
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/admin/tenants/{alice}/credentials/openai"),
        ADMIN,
        Some(json!({"api_key":"synthetic-provider-key","base_url":server.uri()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, bytes) = call(
        &app,
        "POST",
        "/v1/conversations",
        &key,
        Some(json!({"provider":"openai","model":"gpt-6-luna","system":"You are Riley"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let conversation = parsed(&bytes)["id"].as_str().unwrap().to_owned();
    let route = format!("/v1/conversations/{conversation}/turns");
    for text in ["Hello", "Follow up"] {
        let (status, bytes) = call(
            &app,
            "POST",
            &route,
            &key,
            Some(json!({"content":[{"type":"text","text":text}],"stream":true})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let sse = String::from_utf8(bytes).unwrap();
        assert!(sse.contains("turn_ended"));
        assert!(sse.contains("Hello from OpenAI through Loom."));
        assert!(!sse.contains("synthetic-provider-key"));
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].headers["authorization"],
        "Bearer synthetic-provider-key"
    );
    let follow: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(follow["store"], false);
    assert_eq!(
        follow["input"][1]["encrypted_content"],
        "opaque-synthetic-ciphertext"
    );
    assert_eq!(follow["input"][2]["phase"], "final_answer");
    assert_eq!(follow["input"][2]["id"], "msg_synthetic");
    let history = format!("/v1/conversations/{conversation}");
    let (status, bytes) = call(&app, "GET", &history, &key, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parsed(&bytes)["messages"].as_array().unwrap().len(), 4);
    let (status, _) = call(&app, "GET", &history, &bob, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Named references resolve server-side, then capability negotiation refuses
    // the unimplemented provider-hosted MCP path before persisting a user turn.
    let (status, _) = call(
        &app,
        "PUT",
        &format!("/admin/tenants/{alice}/mcp-servers/lucidbrain"),
        ADMIN,
        Some(json!({"url":"https://example.invalid/mcp"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status,_)=call(&app,"POST",&route,&key,Some(json!({"content":[{"type":"text","text":"Do not silently drop tools"}],"options":{"mcp_servers":[{"name":"lucidbrain"}]}}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, bytes) = call(&app, "GET", &history, &key, None).await;
    assert_eq!(parsed(&bytes)["messages"].as_array().unwrap().len(), 4);
}
