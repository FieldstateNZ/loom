//! Boundary/lifecycle checks. Synthetic events do not verify AWS IAM.
use super::*;
use serde_json::{json, Value};
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

fn policy() -> LambdaPolicy {
    LambdaPolicy::new(
        "123456789012".into(),
        "ap-southeast-2".into(),
        vec!["arn:aws:iam::123456789012:role/tandem-test".into()],
    )
    .unwrap()
}

fn envelope(method: &str, path: &str, body: Value) -> Value {
    json!({"version":"2.0","routeKey":"$default","rawPath":path,"rawQueryString":"",
        "headers":{"host":"synthetic.lambda-url.ap-southeast-2.on.aws", "content-type":"application/json",
            "authorization":"AWS4-HMAC-SHA256 synthetic", "x-tandem-authorization":"Bearer synthetic-application"},
        "requestContext":{"accountId":"123456789012", "domainName":"synthetic.lambda-url.ap-southeast-2.on.aws",
            "authorizer":{"iam":{"accountId":"123456789012","userArn":"arn:aws:sts::123456789012:assumed-role/tandem-test/session"}},
            "http":{"method":method,"path":path,"sourceIp":"127.0.0.1","protocol":"HTTP/1.1"}},
        "body":body.to_string(),"isBase64Encoded":false})
}

fn event(value: Value) -> lambda_http::Request {
    let mut context = lambda_http::Context::default();
    context.invoked_function_arn =
        "arn:aws:lambda:ap-southeast-2:123456789012:function:synthetic".into();
    context.deadline = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        + 30_000) as u64;
    lambda_http::request::from_str(&value.to_string())
        .unwrap()
        .with_lambda_context(context)
}

fn state() -> AppState {
    let pool = crate::database::runtime_pool_options(2, true)
        .connect_lazy("postgres://synthetic:synthetic@127.0.0.1:1/synthetic")
        .unwrap();
    AppState::new(
        loom_store::PgStore::new(pool),
        crate::Crypto::new([1; 32]),
        crate::KeyHasher::new(vec![2; 32]),
        "synthetic-admin".into(),
    )
}

#[tokio::test]
async fn function_url_metadata_and_routes_fail_closed_without_opening_database() {
    let gateway = LambdaGateway::from_state(state(), policy());
    assert_eq!(
        gateway
            .handle(event(envelope("GET", "/healthz", json!(null))))
            .await
            .status(),
        StatusCode::OK
    );
    let mut cases = Vec::new();
    for (key, value) in [
        ("accountId", json!("000000000000")),
        ("domainName", json!("evil.invalid")),
        ("authorizer", json!(null)),
    ] {
        let mut e = envelope("GET", "/healthz", json!(null));
        e["requestContext"][key] = value;
        cases.push(e);
    }
    for caller in [
        "arn:aws:sts::123456789012:assumed-role/other/session",
        "arn:aws:sts::000000000000:assumed-role/tandem-test/session",
        "arn:aws:sts::123456789012:assumed-role/tandem-test/session/extra",
    ] {
        let mut e = envelope("GET", "/healthz", json!(null));
        e["requestContext"]["authorizer"]["iam"]["userArn"] = json!(caller);
        cases.push(e);
    }
    let mut cookies = envelope("GET", "/healthz", json!(null));
    cookies["cookies"] = json!(["session=synthetic"]);
    cases.push(cookies);
    let mut query = envelope("GET", "/healthz", json!(null));
    query["rawQueryString"] = json!("admin=true");
    cases.push(query);
    let mut mismatch = envelope("GET", "/healthz", json!(null));
    mismatch["requestContext"]["http"]["path"] = json!("/readyz");
    cases.push(mismatch);
    for e in cases {
        assert_eq!(
            gateway.handle(event(e)).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    for path in [
        "/admin/tenants",
        "/v1/turns",
        "/v1/batches",
        "/openapi.json",
        "/healthz/",
        "/%68ealthz",
    ] {
        assert_eq!(
            gateway
                .handle(event(envelope("POST", path, json!({}))))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let mut huge = envelope("GET", "/healthz", json!(null));
    huge["body"] = json!("x".repeat(MAX_REQUEST + 1));
    assert_eq!(
        gateway.handle(event(huge)).await.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let mut short = event(envelope("GET", "/readyz", json!(null)));
    let mut ctx = short.lambda_context_ref().unwrap().clone();
    ctx.deadline = 1;
    short = short.with_lambda_context(ctx);
    assert_eq!(
        gateway.handle(short).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(gateway.state.request_work_idle());
}

#[tokio::test]
async fn buffered_bytes_preserve_sse_and_only_application_authorization() {
    let mut gateway = LambdaGateway::from_state(state(), policy());
    gateway.schema.set(()).unwrap(); // This boundary-only test performs no SQL.
    gateway.router = Router::new().route(
        "/v1/conversations",
        axum::routing::post(|request: Request<AxumBody>| async move {
            assert_eq!(
                request.headers()["authorization"],
                "Bearer synthetic-application"
            );
            assert!(!request.headers().contains_key("x-tandem-authorization"));
            for name in [
                "x-amz-security-token",
                "host",
                "forwarded",
                "x-forwarded-for",
                "unknown",
            ] {
                assert!(!request.headers().contains_key(name));
            }
            assert_eq!(
                to_bytes(request.into_body(), MAX_REQUEST).await.unwrap(),
                "{\"hello\":\"Riley\"}"
            );
            (
                [("content-type", "text/event-stream")],
                "data: {\"reply\":\"hello\"}\n\n",
            )
        }),
    );
    let mut e = envelope("POST", "/v1/conversations", json!({"hello":"Riley"}));
    for name in [
        "x-amz-security-token",
        "forwarded",
        "x-forwarded-for",
        "unknown",
    ] {
        e["headers"][name] = json!("synthetic-secret");
    }
    let response = gateway.handle(event(e)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.body().as_ref(), b"data: {\"reply\":\"hello\"}\n\n");
    gateway.router = Router::new().route(
        "/healthz",
        axum::routing::get(|| async { "x".repeat(MAX_RESPONSE + 1) }),
    );
    assert_eq!(
        gateway
            .handle(event(envelope("GET", "/healthz", json!(null))))
            .await
            .status(),
        StatusCode::BAD_GATEWAY
    );
}

#[tokio::test]
async fn dropping_caller_keeps_admission_until_registered_cleanup_finishes() {
    let mut gateway = LambdaGateway::from_state(state(), policy());
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let handler_state = gateway.state.clone();
    let started = entered.clone();
    let finish = release.clone();
    gateway.router = Router::new().route(
        "/healthz",
        axum::routing::get(move || {
            let cleanup = handler_state.track_cleanup();
            let started = started.clone();
            let finish = finish.clone();
            async move {
                tokio::spawn(async move {
                    started.notify_one();
                    finish.notified().await;
                    cleanup.complete();
                });
                "ok"
            }
        }),
    );
    let cloned = gateway.clone();
    let caller = tokio::spawn(async move {
        cloned
            .handle(event(envelope("GET", "/healthz", json!(null))))
            .await
    });
    entered.notified().await;
    caller.abort();
    assert!(!gateway.state.request_work_idle());
    assert_eq!(
        gateway
            .handle(event(envelope("GET", "/healthz", json!(null))))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        while gateway.admission.available_permits() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(gateway.state.request_work_idle());
}

#[tokio::test]
async fn timed_out_work_is_not_retried_and_failed_cleanup_retires_environment() {
    let mut gateway = LambdaGateway::from_state(state(), policy());
    gateway.work_budget = Duration::from_millis(20);
    gateway.cleanup_budget = Duration::from_millis(30);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    gateway.router = Router::new().route(
        "/healthz",
        axum::routing::get(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { std::future::pending::<&'static str>().await }
        }),
    );
    assert_eq!(
        gateway
            .handle(event(envelope("GET", "/healthz", json!(null))))
            .await
            .status(),
        StatusCode::GATEWAY_TIMEOUT
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(gateway.state.request_work_idle());
    let retired = Arc::new(AtomicUsize::new(0));
    let count = retired.clone();
    gateway.retire = Arc::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
    });
    // An aborted settlement task is sticky, even after its counter reaches zero.
    drop(gateway.state.track_cleanup());
    assert_eq!(
        gateway
            .handle(event(envelope("GET", "/healthz", json!(null))))
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(retired.load(Ordering::SeqCst), 1);
    assert_eq!(
        gateway
            .handle(event(envelope("GET", "/healthz", json!(null))))
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(retired.load(Ordering::SeqCst), 1);
}

struct Factory(Arc<TestProvider>);
#[async_trait::async_trait]
impl crate::ProviderFactory for Factory {
    async fn provider(
        &self,
        _: &AppState,
        _: Uuid,
        _: &str,
    ) -> Result<Arc<dyn loom_provider::Provider>, crate::ApiError> {
        Ok(self.0.clone())
    }
}
struct TestProvider {
    inner: loom_provider::mock::MockProvider,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl loom_provider::Provider for TestProvider {
    fn descriptor(&self) -> loom_provider::ProviderDescriptor {
        self.inner.descriptor()
    }
    async fn complete(
        &self,
        c: &loom_core::Conversation,
        o: &loom_core::ConversationOptions,
    ) -> Result<loom_core::Message, loom_provider::ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.complete(c, o).await
    }
    async fn stream(
        &self,
        c: &loom_core::Conversation,
        o: &loom_core::ConversationOptions,
    ) -> Result<loom_provider::TurnEventStream, loom_provider::ProviderError> {
        use futures::StreamExt;
        self.calls.fetch_add(1, Ordering::SeqCst);
        // Emit the real runner's started frame then stall until the adapter's
        // budget drops its response body. No paid provider or MCP is involved.
        Ok(Box::pin(
            self.inner
                .stream(c, o)
                .await?
                .take(1)
                .chain(futures::stream::pending()),
        ))
    }
    fn count_cost(&self, u: &loom_core::Usage, m: &str) -> loom_provider::Cost {
        self.inner.count_cost(u, m)
    }
}
struct DelayedRecorder {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
#[async_trait::async_trait]
impl crate::UsageRecorder for DelayedRecorder {
    async fn record(&self, store: &loom_store::PgStore, event: loom_store::NewUsageEvent) {
        self.entered.notify_one();
        self.release.notified().await;
        crate::UsageRecorder::record(&crate::OutboxUsageRecorder, store, event).await;
    }
}

#[tokio::test]
async fn actual_router_postgres_cancelled_stream_settles_before_ack_and_reconnects() {
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;
    let container = Postgres::default().with_tag("16").start().await.unwrap();
    let url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        container.get_host_port_ipv4(5432).await.unwrap()
    );
    let observer = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    loom_store::run_migrations(&observer).await.unwrap();
    let pool = crate::database::runtime_pool_options(2, true)
        .connect_lazy(&format!("{url}?application_name=loom-lambda-test"))
        .unwrap();
    let provider = Arc::new(TestProvider {
        inner: loom_provider::mock::MockProvider::new(
            "anthropic",
            "claude-opus-4-8",
            [loom_provider::Capability::Streaming],
        ),
        calls: AtomicUsize::new(0),
    });
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let state = AppState::new(
        loom_store::PgStore::new(pool),
        crate::Crypto::new([1; 32]),
        crate::KeyHasher::new(vec![2; 32]),
        "synthetic-admin".into(),
    )
    .with_provider_factory(Arc::new(Factory(provider.clone())))
    .with_usage_recorder(Arc::new(DelayedRecorder {
        entered: entered.clone(),
        release: release.clone(),
    }));
    let admin = build_router(state.clone());
    async fn provision(router: &Router, path: &str, body: Value) -> Value {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("authorization", "Bearer synthetic-admin")
                    .header("content-type", "application/json")
                    .body(AxumBody::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        serde_json::from_slice(&to_bytes(response.into_body(), MAX_RESPONSE).await.unwrap())
            .unwrap()
    }
    let tenant = provision(
        &admin,
        "/admin/tenants",
        json!({"slug":"lambda-test","name":"Synthetic"}),
    )
    .await;
    let key = provision(
        &admin,
        &format!("/admin/tenants/{}/keys", tenant["id"].as_str().unwrap()),
        json!({"name":"synthetic","env":"test"}),
    )
    .await;
    let mut gateway = LambdaGateway::from_state(state, policy());
    let make = |method, path: &str, body| {
        let mut e = envelope(method, path, body);
        e["headers"]["x-tandem-authorization"] =
            json!(format!("Bearer {}", key["key"].as_str().unwrap()));
        event(e)
    };
    let response = gateway
        .handle(make(
            "POST",
            "/v1/conversations",
            json!({"provider":"anthropic","model":"claude-opus-4-8"}),
        ))
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let conversation: Value = serde_json::from_slice(response.body().as_ref()).unwrap();
    let id = conversation["id"].as_str().unwrap();
    assert!(gateway.state.request_work_idle());
    let sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE application_name='loom-lambda-test'",
    )
    .fetch_one(&observer)
    .await
    .unwrap();
    assert_eq!(sessions, 0);
    gateway.work_budget = Duration::from_secs(2);
    let request = make(
        "POST",
        &format!("/v1/conversations/{id}/turns"),
        json!({"content":[{"type":"text","text":"hello"}],"stream":true}),
    );
    let clone = gateway.clone();
    let mut invocation = tokio::spawn(async move { clone.handle(request).await });
    tokio::select! {
        _ = entered.notified() => {},
        response = &mut invocation => panic!("stream did not reach settlement: {:?}; provider calls {}", response, provider.calls.load(Ordering::SeqCst)),
        _ = tokio::time::sleep(Duration::from_secs(4)) => panic!("settlement never started"),
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !invocation.is_finished(),
        "partial settlement must delay acknowledgement"
    );
    assert!(!gateway.state.request_work_idle());
    release.notify_one();
    let response = invocation.await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert!(gateway.state.request_work_idle());
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM usage_events")
        .fetch_one(&observer)
        .await
        .unwrap();
    assert_eq!(
        rows, 1,
        "exactly one settlement, no automatic inference retry"
    );
    let sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE application_name='loom-lambda-test'",
    )
    .fetch_one(&observer)
    .await
    .unwrap();
    assert_eq!(
        sessions, 0,
        "actual sessions must close before ack, not just pool checkout counters"
    );
    gateway.work_budget = Duration::from_secs(3);
    let response = gateway
        .handle(make("GET", &format!("/v1/conversations/{id}"), json!(null)))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert!(gateway.state.request_work_idle());
    gateway.state.store().pool().close().await;
    observer.close().await;
}
