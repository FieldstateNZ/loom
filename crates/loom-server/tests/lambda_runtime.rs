//! The unmodified official Rust Runtime API loop, driven by a local HTTP peer.
//! This does not establish AWS IAM, cloud cold-start or ARM64 image readiness.
use axum::{
    body::{to_bytes, Body},
    extract::{Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;

#[derive(Clone)]
struct Runtime {
    next: Arc<AtomicUsize>,
    results: mpsc::UnboundedSender<(String, Value)>,
}
async fn next(State(state): State<Runtime>) -> Response {
    let index = state.next.fetch_add(1, Ordering::SeqCst);
    if index > 1 {
        return std::future::pending().await;
    }
    let path = if index == 0 {
        "/healthz"
    } else {
        "/admin/tenants"
    };
    let deadline = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        + 30_000;
    let event = json!({"version":"2.0","routeKey":"$default","rawPath":path,"rawQueryString":"",
        "headers":{"host":"synthetic.lambda-url.ap-southeast-2.on.aws"},
        "requestContext":{"accountId":"123456789012","domainName":"synthetic.lambda-url.ap-southeast-2.on.aws",
            "authorizer":{"iam":{"accountId":"123456789012","userArn":"arn:aws:sts::123456789012:assumed-role/tandem-test/session"}},
            "http":{"method":"GET","path":path,"sourceIp":"127.0.0.1","protocol":"HTTP/1.1"}},
        "isBase64Encoded":false});
    Response::builder()
        .header(
            "lambda-runtime-aws-request-id",
            format!("synthetic-{index}"),
        )
        .header("lambda-runtime-deadline-ms", deadline.to_string())
        .header(
            "lambda-runtime-invoked-function-arn",
            "arn:aws:lambda:ap-southeast-2:123456789012:function:synthetic",
        )
        .body(Body::from(event.to_string()))
        .unwrap()
}
async fn result(
    State(state): State<Runtime>,
    Path((id, kind)): Path<(String, String)>,
    request: Request<Body>,
) -> impl IntoResponse {
    let bytes = to_bytes(request.into_body(), 512 * 1024).await.unwrap();
    let payload = serde_json::from_slice(&bytes).unwrap();
    state
        .results
        .send((format!("{id}/{kind}"), payload))
        .unwrap();
    StatusCode::ACCEPTED
}

#[tokio::test]
async fn official_runtime_converts_function_url_events_and_posts_buffered_responses() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/2018-06-01/runtime/invocation/next", get(next))
        .route("/2018-06-01/runtime/invocation/{id}/{kind}", post(result))
        .with_state(Runtime {
            next: Arc::new(AtomicUsize::new(0)),
            results: tx,
        });
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_loom-lambda"))
        .env_clear()
        .env("AWS_LAMBDA_RUNTIME_API", address.to_string())
        .env("AWS_REGION", "ap-southeast-2")
        .env("AWS_LAMBDA_FUNCTION_NAME", "synthetic")
        .env("AWS_LAMBDA_FUNCTION_MEMORY_SIZE", "256")
        .env("AWS_LAMBDA_FUNCTION_VERSION", "$LATEST")
        .env(
            "DATABASE_URL",
            "postgres://synthetic:synthetic@127.0.0.1:1/synthetic",
        )
        .env("LOOM_REQUEST_DRIVEN", "true")
        .env("LOOM_ROOT_ADMIN_TOKEN", "synthetic-admin")
        .env("LOOM_ENCRYPTION_KEY", "01".repeat(32))
        .env("LOOM_LAMBDA_ACCOUNT_ID", "123456789012")
        .env(
            "LOOM_LAMBDA_CALLER_ROLE_ARNS",
            "[\"arn:aws:iam::123456789012:role/tandem-test\"]",
        )
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    for expected in [200, 404] {
        let (id, response) = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(id.ends_with("/response"));
        assert_eq!(response["statusCode"], expected);
        if expected == 200 {
            // The runtime may encode text media as UTF-8 instead of base64.
            use base64::Engine;
            let body = if response["isBase64Encoded"].as_bool().unwrap() {
                base64::engine::general_purpose::STANDARD
                    .decode(response["body"].as_str().unwrap())
                    .unwrap()
            } else {
                response["body"].as_str().unwrap().as_bytes().to_vec()
            };
            assert_eq!(body, b"ok");
        }
    }
    assert!(
        child.try_wait().unwrap().is_none(),
        "one environment must serve successive events"
    );
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    server.abort();
}
