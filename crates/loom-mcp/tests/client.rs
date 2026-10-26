use loom_mcp::{Binding, MemoryClient};
use serde_json::{json, Value};
use uuid::Uuid;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn binding() -> Binding {
    Binding {
        principal_id: Uuid::new_v4(),
        workspace_id: Uuid::new_v4(),
        agent_definition_id: Uuid::new_v4(),
        credential_id: Uuid::new_v4(),
    }
}
fn context(b: &Binding) -> Value {
    let mut value = serde_json::to_value(b).unwrap();
    let o = value.as_object_mut().unwrap();
    o.insert("schema_version".into(), json!(1));
    o.insert("kind".into(), json!("workspace_assistant"));
    o.insert("allowed_tools".into(), json!(["memory_search"]));
    o.insert("transport_mode".into(), json!("stateless"));
    value
}

async fn fixture(ctx: Value, call_status: u16, result: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/mcp/context"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ctx))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(405))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/mcp")).and(body_partial_json(json!({"method":"initialize"})))
        .respond_with(|r:&Request|{let req:Value=serde_json::from_slice(&r.body).unwrap();ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":req["id"],
            "result":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"lucidbrain","version":"1.0.0"}}}))}).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(
            json!({"method":"notifications/initialized"}),
        ))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/mcp")).and(body_partial_json(json!({"method":"tools/list"})))
        .respond_with(|r:&Request|{let req:Value=serde_json::from_slice(&r.body).unwrap();ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":req["id"],
            "result":{"tools":[{"name":"memory_search","description":"Browse memories","inputSchema":{"type":"object","properties":{"search":{"type":"string"}}}}]}}))}).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(json!({"method":"tools/call"})))
        .respond_with(move |r: &Request| {
            let req: Value = serde_json::from_slice(&r.body).unwrap();
            ResponseTemplate::new(call_status)
                .set_body_json(json!({"jsonrpc":"2.0","id":req["id"],"result":result}))
        })
        .mount(&server)
        .await;
    server
}
async fn connect(server: &MockServer, b: Binding) -> Result<MemoryClient, loom_mcp::Error> {
    MemoryClient::connect(
        &format!("{}/mcp", server.uri()),
        "synthetic-private-token",
        b,
        &["memory_search".into()],
        &["memory_search".into()],
    )
    .await
}

#[tokio::test]
async fn official_sdk_handshake_discovery_and_read_are_identity_bound() {
    let b = binding();
    let server = fixture(
        context(&b),
        200,
        json!({"content":[{"type":"text","text":"{\"memories\":[\"blue tea\"]}"}]}),
    )
    .await;
    let client = connect(&server, b.clone()).await.unwrap();
    assert_eq!(client.tools().len(), 1);
    assert!(!serde_json::to_string(client.tools())
        .unwrap()
        .contains("synthetic-private-token"));
    let (value, error) = client
        .call("memory_search", &json!({"search":"tea"}))
        .await
        .unwrap();
    assert!(!error);
    assert!(value.to_string().contains("blue tea"));
    let requests = server.received_requests().await.unwrap();
    for r in requests {
        assert_eq!(r.headers["authorization"], "Bearer synthetic-private-token");
        assert_eq!(
            r.headers["x-lucidbrain-expected-workspace"],
            b.workspace_id.to_string()
        );
        assert_eq!(
            r.headers["x-lucidbrain-expected-principal"],
            b.principal_id.to_string()
        );
        assert_eq!(
            r.headers["x-lucidbrain-expected-agent"],
            b.agent_definition_id.to_string()
        );
        assert_eq!(
            r.headers["x-lucidbrain-expected-credential"],
            b.credential_id.to_string()
        );
    }
}

#[tokio::test]
async fn identity_and_unrestricted_or_stateful_policy_mismatches_fail_before_mcp() {
    let b = binding();
    for (field, value) in [
        ("workspace_id", json!(Uuid::new_v4())),
        ("principal_id", json!(Uuid::new_v4())),
        ("agent_definition_id", json!(Uuid::new_v4())),
        ("credential_id", json!(Uuid::new_v4())),
        ("allowed_tools", Value::Null),
        ("allowed_tools", json!(["capture_item"])),
        ("transport_mode", json!("stateful")),
        ("schema_version", json!(2)),
    ] {
        let mut ctx = context(&b);
        ctx[field] = value;
        let server = fixture(ctx, 200, json!({})).await;
        assert!(connect(&server, b.clone()).await.is_err());
        assert!(server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET"));
    }
}

#[tokio::test]
async fn write_tools_unknown_arguments_and_invalid_limits_never_dispatch() {
    let b = binding();
    let server = fixture(context(&b), 200, json!({"content":[]})).await;
    let client = connect(&server, b).await.unwrap();
    for (name, args) in [
        ("capture_item", json!({"title":"No"})),
        ("memory_search", json!({"workspaceId":Uuid::new_v4()})),
        ("memory_search", json!({"limit":-1})),
    ] {
        assert!(client.call(name, &args).await.is_err());
    }
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| !String::from_utf8_lossy(&r.body).contains("tools/call")));
}

#[tokio::test]
async fn failed_tool_call_is_not_retried_and_sdk_errors_are_private() {
    let b = binding();
    let server = fixture(
        context(&b),
        503,
        json!({"secret":"synthetic-private-token"}),
    )
    .await;
    let client = connect(&server, b).await.unwrap();
    let error = client.call("memory_search", &json!({})).await.unwrap_err();
    assert!(!format!("{error:?} {error}").contains("synthetic-private-token"));
    let count = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| String::from_utf8_lossy(&r.body).contains("tools/call"))
        .count();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn tool_error_payloads_are_not_forwarded_to_the_model() {
    let b = binding();
    let server=fixture(context(&b),200,json!({"isError":true,"content":[{"type":"text","text":"synthetic-private-token diagnostic"}]})).await;
    let client = connect(&server, b).await.unwrap();
    let (value, error) = client.call("memory_search", &json!({})).await.unwrap();
    assert!(error);
    assert!(!value.to_string().contains("synthetic-private-token"));
}

#[tokio::test]
async fn redirects_cannot_receive_the_private_credential() {
    let target = MockServer::start().await;
    let source = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/mcp/context"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/mcp/context", target.uri())),
        )
        .mount(&source)
        .await;
    assert!(connect(&source, binding()).await.is_err());
    assert!(target.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn unsafe_endpoints_and_unregistered_tool_requests_are_rejected() {
    for url in [
        "http://private.example/mcp",
        "https://user:secret@example.com/mcp",
        "https://example.com/mcp?token=x",
        "https://example.com/other",
    ] {
        assert!(MemoryClient::connect(
            url,
            "synthetic",
            binding(),
            &["memory_search".into()],
            &["memory_search".into()]
        )
        .await
        .is_err());
    }
    let server = MockServer::start().await;
    assert!(MemoryClient::connect(
        &format!("{}/mcp", server.uri()),
        "synthetic",
        binding(),
        &["memory_search".into()],
        &["memory_recall".into()]
    )
    .await
    .is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
}
