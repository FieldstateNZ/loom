//! Real gateway, encrypted credentials, migrations and PostgreSQL. Protocol
//! peers are mocked here; Tandem's joint check uses the actual LucidBrain MCP.
use axum::{body::Body, Router};
use http::{Request, StatusCode};
use loom_server::{build_router, AppState, Crypto, KeyHasher};
use loom_store::PgStore;
use serde_json::{json, Value};
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, Request as MockRequest, ResponseTemplate,
};

const ADMIN: &str = "synthetic-gateway-admin";
async fn request(
    app: &Router,
    method: &str,
    url: &str,
    key: &str,
    body: Option<Value>,
) -> axum::response::Response {
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
    app.clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}
async fn call(
    app: &Router,
    method: &str,
    url: &str,
    key: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let response = request(app, method, url, key, body).await;
    let status = response.status();
    (
        status,
        axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
}
fn parsed(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}
async fn json_call(app: &Router, method: &str, url: &str, key: &str, body: Value) -> Value {
    let (status, bytes) = call(app, method, url, key, Some(body)).await;
    assert!(
        status.is_success(),
        "status={status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    parsed(&bytes)
}
async fn identity(app: &Router) -> (String, String) {
    let tenant = json_call(
        app,
        "POST",
        "/admin/tenants",
        ADMIN,
        json!({"slug":format!("gateway-{}",Uuid::new_v4()),"name":"Synthetic"}),
    )
    .await;
    let id = tenant["id"].as_str().unwrap().to_owned();
    let key = json_call(
        app,
        "POST",
        &format!("/admin/tenants/{id}/keys"),
        ADMIN,
        json!({"name":"synthetic","env":"test"}),
    )
    .await;
    (id, key["key"].as_str().unwrap().into())
}

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

struct Peer {
    binding: Mutex<Value>,
    category: String,
    category_name: Mutex<String>,
    captured: Mutex<HashMap<String, Value>>,
    captures: AtomicUsize,
    lists: AtomicUsize,
    lost_reply: AtomicBool,
    slow_capture: AtomicBool,
    slow_category: AtomicBool,
    receipts_only: AtomicBool,
    bad_proposal: AtomicBool,
}
struct Fixture {
    app: Router,
    state: AppState,
    tenant: String,
    key: String,
    peer: Arc<Peer>,
    mcp: MockServer,
    model: MockServer,
    binding: Value,
    _pg: ContainerAsync<Postgres>,
}
const TOOLS: [&str; 4] = [
    "memory_search",
    "list_categories",
    "capture_item",
    "get_capture_operation",
];
impl Fixture {
    async fn new() -> Self {
        let pg = Postgres::default().with_tag("16").start().await.unwrap();
        let port = pg.get_host_port_ipv4(5432).await.unwrap();
        let store = PgStore::connect(&format!(
            "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
        ))
        .await
        .unwrap();
        loom_store::run_migrations(store.pool()).await.unwrap();
        let state = AppState::new(
            store,
            Crypto::new([0x61; 32]),
            KeyHasher::new(b"synthetic-gateway-pepper".to_vec()),
            ADMIN.into(),
        );
        let app = build_router(state.clone());
        let (tenant, key) = identity(&app).await;
        let mcp = MockServer::start().await;
        let model = MockServer::start().await;
        let binding = json!({"principal_id":Uuid::new_v4(),"workspace_id":Uuid::new_v4(),"agent_definition_id":Uuid::new_v4(),"credential_id":Uuid::new_v4()});
        let peer = Arc::new(Peer {
            binding: Mutex::new(binding.clone()),
            category: Uuid::new_v4().to_string(),
            category_name: Mutex::new("Personal".into()),
            captured: Mutex::new(HashMap::new()),
            captures: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            lost_reply: AtomicBool::new(false),
            slow_capture: AtomicBool::new(false),
            slow_category: AtomicBool::new(false),
            receipts_only: AtomicBool::new(false),
            bad_proposal: AtomicBool::new(false),
        });
        let p = peer.clone();
        Mock::given(method("GET"))
            .and(path("/mcp/context"))
            .respond_with(move |_: &MockRequest| {
                let mut c = p.binding.lock().unwrap().clone();
                c["schema_version"] = json!(1);
                c["capture_operation_version"] = json!(1);
                c["kind"] = json!("workspace_assistant");
                c["transport_mode"] = json!("stateless");
                c["allowed_tools"] = if p.receipts_only.load(Ordering::SeqCst) {
                    json!(["get_capture_operation"])
                } else {
                    json!(TOOLS)
                };
                ResponseTemplate::new(200).set_body_json(c)
            })
            .mount(&mcp)
            .await;
        Mock::given(method("GET"))
            .and(path("/mcp"))
            .respond_with(ResponseTemplate::new(405))
            .mount(&mcp)
            .await;
        let p = peer.clone();
        Mock::given(method("POST")).and(path("/mcp")).respond_with(move|r:&MockRequest|{
            let q:Value=serde_json::from_slice(&r.body).unwrap();
            let expected=p.binding.lock().unwrap().clone();
            for (header,field) in [("x-lucidbrain-expected-principal","principal_id"),("x-lucidbrain-expected-workspace","workspace_id"),("x-lucidbrain-expected-agent","agent_definition_id"),("x-lucidbrain-expected-credential","credential_id")] {
                assert_eq!(r.headers.get(header).unwrap().to_str().unwrap(),expected[field].as_str().unwrap());
            }
            let mut delay=false;
            let result=match q["method"].as_str().unwrap() {
                "notifications/initialized"=>return ResponseTemplate::new(202),
                "initialize"=>json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"lucidbrain","version":"1.0.0"}}),
                "tools/list"=>json!({"tools":TOOLS.iter().filter(|n|!p.receipts_only.load(Ordering::SeqCst)||**n=="get_capture_operation").map(|n|json!({"name":n,"inputSchema":{"type":"object","properties":{}}})).collect::<Vec<_>>()}),
                "tools/call"=>{
                    let args=&q["params"]["arguments"];
                    let value=match q["params"]["name"].as_str().unwrap(){
                        "list_categories"=>{p.lists.fetch_add(1,Ordering::SeqCst);assert_eq!(args["workspaceSlug"],"synthetic-workspace");delay=p.slow_category.load(Ordering::SeqCst);json!([{"id":p.category,"workspaceId":expected["workspace_id"],"displayName":p.category_name.lock().unwrap().clone(),"archived":false}])},
                        "capture_item"=>{
                            p.captures.fetch_add(1,Ordering::SeqCst);
                            assert_eq!(args["title"],"Call the builder");assert_eq!(args["categoryId"],p.category);
                            assert!(args.get("workspaceSlug").is_none());
                            let id=args["operationId"].as_str().unwrap();Uuid::parse_str(id).unwrap();
                            let mut saved=p.captured.lock().unwrap();
                            let receipt=saved.entry(id.into()).or_insert_with(||json!({"operationId":id,"itemId":Uuid::new_v4(),"workspaceId":expected["workspace_id"],"committedAt":"2026-10-27T00:00:00Z","status":"committed"})).clone();
                            if p.lost_reply.swap(false,Ordering::SeqCst){return ResponseTemplate::new(503);}
                            delay=p.slow_capture.load(Ordering::SeqCst);receipt
                        },
                        "get_capture_operation"=>p.captured.lock().unwrap().get(args["operationId"].as_str().unwrap()).cloned().unwrap_or_else(||json!({"operationId":args["operationId"],"status":"not_observed"})),
                        _=>panic!("unreviewed tool")
                    };
                    json!({"content":[{"type":"text","text":value.to_string()}]})
                }, _=>panic!("unexpected MCP request")
            };
            let response=ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":q["id"],"result":result}));
            if delay{response.set_delay(Duration::from_millis(800))}else{response}
        }).mount(&mcp).await;
        let p = peer.clone();
        let hidden = mcp.uri();
        Mock::given(method("POST")).and(path("/v1/responses")).respond_with(move|r:&MockRequest|{
            let q:Value=serde_json::from_slice(&r.body).unwrap();let raw=String::from_utf8_lossy(&r.body);
            assert!(!raw.contains("synthetic-private-mcp-token")&&!raw.contains(&hidden));
            let names:Vec<_>=q["tools"].as_array().unwrap().iter().map(|v|v["name"].as_str().unwrap()).collect();
            assert!(names.contains(&"propose_task"));assert!(!names.contains(&"capture_item"));assert!(!names.contains(&"get_capture_operation"));
            let done=q["input"].as_array().unwrap().last().is_some_and(|v|v["type"]=="function_call_output");
            let id=Uuid::new_v4().simple().to_string();
            let output=if done {assert!(raw.contains("requires_application_approval"));json!([{"type":"message","id":format!("msg_{id}"),"role":"assistant","status":"completed","phase":"final_answer","content":[{"type":"output_text","text":"The task is ready for your confirmation.","annotations":[]}]}])}
                else{let mut args=json!({"title":"Call the builder","detail":"Ask about the kitchen"});if p.bad_proposal.load(Ordering::SeqCst){args["operationId"]=json!(Uuid::new_v4());}json!([{"type":"function_call","id":format!("fc_{id}"),"call_id":format!("call_{id}"),"name":"propose_task","arguments":args.to_string(),"status":"completed"}])};
            let event=json!({"type":"response.completed","response":{"id":format!("resp_{id}"),"status":"completed","model":"gpt-6-luna","output":output,"usage":{"input_tokens":10,"output_tokens":8}}});
            let created=json!({"type":"response.created","response":{"id":format!("resp_{id}"),"status":"in_progress"}});
            ResponseTemplate::new(200).set_body_raw(format!("event: response.created\ndata: {created}\n\nevent: response.completed\ndata: {event}\n\n"),"text/event-stream")
        }).mount(&model).await;
        json_call(
            &app,
            "PUT",
            &format!("/admin/tenants/{tenant}/credentials/openai"),
            ADMIN,
            json!({"api_key":"synthetic-openai-key","base_url":model.uri()}),
        )
        .await;
        let f = Self {
            app,
            state,
            tenant,
            key,
            peer,
            mcp,
            model,
            binding,
            _pg: pg,
        };
        f.register().await;
        f
    }
    async fn register(&self) {
        let tools = if self.peer.receipts_only.load(Ordering::SeqCst) {
            json!(["get_capture_operation"])
        } else {
            json!(TOOLS)
        };
        json_call(&self.app,"PUT",&format!("/admin/tenants/{}/mcp-servers/lucidbrain",self.tenant),ADMIN,json!({"url":format!("{}/mcp",self.mcp.uri()),"authorization_token":"synthetic-private-mcp-token","tool_configuration":{"enabled":true,"allowed_tools":tools,"gateway":{"kind":"lucidbrain_memory","binding":self.peer.binding.lock().unwrap().clone(),"task_capture":{"workspace_slug":"synthetic-workspace","category_id":self.peer.category,"provenance":"real_world"}}}})).await;
    }
    async fn conversation(&self) -> String {
        json_call(&self.app,"POST","/v1/conversations",&self.key,json!({"provider":"openai","model":"gpt-6-luna","system":"You are Riley. Proposals require application approval."})).await["id"].as_str().unwrap().into()
    }
    fn turn(&self, provenance: &str) -> Value {
        json!({"content":[{"type":"text","text":"Create a task to call the builder"}],"stream":true,"gateway_mcp":{"name":"lucidbrain","allowed_tools":["memory_search"],"expected_binding":self.binding,"propose_tasks":true,"provenance":provenance}})
    }
    fn path(id: &str, action: &Value, command: &str) -> String {
        format!(
            "/v1/conversations/{id}/actions/{}/{command}",
            action["id"].as_str().unwrap()
        )
    }
    async fn proposal(&self) -> (String, Value) {
        let id = self.conversation().await;
        let (s, b) = call(
            &self.app,
            "POST",
            &format!("/v1/conversations/{id}/turns"),
            &self.key,
            Some(self.turn("real_world")),
        )
        .await;
        assert_eq!(s, 200, "{}", String::from_utf8_lossy(&b));
        let text = String::from_utf8(b).unwrap();
        assert!(
            text.contains("loom_action_proposed") && text.contains("turn_ended"),
            "{text}"
        );
        assert!(!text.contains("synthetic-private-mcp-token") && !text.contains(&self.mcp.uri()));
        let (s, b) = call(
            &self.app,
            "GET",
            &format!("/v1/conversations/{id}/actions"),
            &self.key,
            None,
        )
        .await;
        assert_eq!(s, 200);
        let list = parsed(&b);
        assert_eq!(list.as_array().unwrap().len(), 1);
        let a = list[0].clone();
        assert_eq!(a["status"], "pending");
        assert_eq!(a["proposal"]["input"]["title"], "Call the builder");
        assert!(self.peer.captured.lock().unwrap().is_empty());
        (id, a)
    }
    async fn approve(&self, id: &str, a: &Value) -> Value {
        json_call(
            &self.app,
            "POST",
            &Self::path(id, a, "decision"),
            &self.key,
            json!({"decision":"approve","approval_digest":a["approval_digest"]}),
        )
        .await
    }
    async fn command(&self, id: &str, a: &Value, command: &str) -> Value {
        json_call(
            &self.app,
            "POST",
            &Self::path(id, a, command),
            &self.key,
            json!({}),
        )
        .await
    }
}

#[tokio::test]
async fn proposal_approval_and_dispatch_are_distinct_durable_steps() {
    let f = Fixture::new().await;
    let (id, a) = f.proposal().await;
    let (s, _) = call(
        &f.app,
        "POST",
        &Fixture::path(&id, &a, "dispatch"),
        &f.key,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 409);
    let (s, _) = call(
        &f.app,
        "POST",
        &Fixture::path(&id, &a, "decision"),
        &f.key,
        Some(json!({"decision":"approve","approval_digest":"wrong"})),
    )
    .await;
    assert_eq!(s, 409);
    assert_eq!(f.approve(&id, &a).await["status"], "approved");
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 0);
    let done = f.command(&id, &a, "dispatch").await;
    assert_eq!(done["status"], "completed");
    assert_eq!(done["receipt"]["operationId"], a["id"]);
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.command(&id, &a, "dispatch").await["receipt"],
        done["receipt"]
    );
    assert_eq!(
        f.command(&id, &a, "reconcile").await["receipt"],
        done["receipt"]
    );
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 1);
    let (_, bob) = identity(&f.app).await;
    let (s, _) = call(
        &f.app,
        "GET",
        &format!("/v1/conversations/{id}/actions"),
        &bob,
        None,
    )
    .await;
    assert_eq!(s, 404);
    let (s, _) = call(
        &f.app,
        "POST",
        &Fixture::path(&id, &a, "decision"),
        &bob,
        Some(json!({"decision":"approve","approval_digest":a["approval_digest"]})),
    )
    .await;
    assert_eq!(s, 404);
    let (s, b) = call(
        &f.app,
        "GET",
        &format!("/v1/conversations/{id}"),
        &f.key,
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(
        String::from_utf8(b)
            .unwrap()
            .matches("Application-confirmed task result")
            .count(),
        1
    );
}

#[tokio::test]
async fn rejected_canceled_and_expired_approvals_never_dispatch() {
    let f = Fixture::new().await;
    for state in ["rejected", "canceled", "expired"] {
        let (id, a) = f.proposal().await;
        if state == "rejected" {
            json_call(
                &f.app,
                "POST",
                &Fixture::path(&id, &a, "decision"),
                &f.key,
                json!({"decision":"reject","approval_digest":a["approval_digest"]}),
            )
            .await;
        } else {
            f.approve(&id, &a).await;
            if state == "canceled" {
                f.command(&id, &a, "cancel").await;
            } else {
                sqlx::query(
                    "UPDATE gateway_actions SET expires_at=now()-interval '1 second' WHERE id=$1",
                )
                .bind(Uuid::parse_str(a["id"].as_str().unwrap()).unwrap())
                .execute(f.state.store().pool())
                .await
                .unwrap();
            }
        }
        let (s, _) = call(
            &f.app,
            "POST",
            &Fixture::path(&id, &a, "dispatch"),
            &f.key,
            Some(json!({})),
        )
        .await;
        assert_eq!(s, 409);
    }
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn lost_committed_reply_reconciles_with_read_only_replacement_credential() {
    let f = Fixture::new().await;
    let (id, a) = f.proposal().await;
    f.approve(&id, &a).await;
    f.peer.lost_reply.store(true, Ordering::SeqCst);
    assert_eq!(f.command(&id, &a, "dispatch").await["status"], "uncertain");
    assert_eq!(f.peer.captured.lock().unwrap().len(), 1);
    let (s, _) = call(
        &f.app,
        "DELETE",
        &format!("/v1/conversations/{id}"),
        &f.key,
        None,
    )
    .await;
    assert_eq!(s, 409);
    f.peer.binding.lock().unwrap()["credential_id"] = json!(Uuid::new_v4());
    f.peer.receipts_only.store(true, Ordering::SeqCst);
    f.register().await;
    let restarted = build_router(f.state.clone());
    let done = json_call(
        &restarted,
        "POST",
        &Fixture::path(&id, &a, "reconcile"),
        &f.key,
        json!({}),
    )
    .await;
    assert_eq!(done["status"], "completed");
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 1);
}

async fn until(mut predicate: impl FnMut() -> bool) {
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(tokio::time::Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn cancellation_wins_during_dispatch_preflight_without_a_write() {
    let f = Fixture::new().await;
    let (id, a) = f.proposal().await;
    f.approve(&id, &a).await;
    f.peer.slow_category.store(true, Ordering::SeqCst);
    let app = f.app.clone();
    let key = f.key.clone();
    let path = Fixture::path(&id, &a, "dispatch");
    let running =
        tokio::spawn(async move { call(&app, "POST", &path, &key, Some(json!({}))).await });
    until(|| f.peer.lists.load(Ordering::SeqCst) >= 2).await;
    assert_eq!(f.command(&id, &a, "cancel").await["status"], "canceled");
    let (s, b) = running.await.unwrap();
    assert_eq!(s, 200);
    assert_eq!(parsed(&b)["status"], "canceled");
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancel_after_dispatch_preserves_committed_truth_and_denies_overlap() {
    let f = Fixture::new().await;
    let (id, a) = f.proposal().await;
    f.approve(&id, &a).await;
    f.peer.slow_capture.store(true, Ordering::SeqCst);
    let app = f.app.clone();
    let key = f.key.clone();
    let path = Fixture::path(&id, &a, "dispatch");
    let running =
        tokio::spawn(async move { call(&app, "POST", &path, &key, Some(json!({}))).await });
    until(|| f.peer.captures.load(Ordering::SeqCst) == 1).await;
    let (s, _) = call(
        &f.app,
        "POST",
        &Fixture::path(&id, &a, "dispatch"),
        &f.key,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 409);
    let canceled = f.command(&id, &a, "cancel").await;
    assert_eq!(canceled["status"], "dispatched");
    assert_eq!(canceled["cancel_requested"], true);
    let (s, b) = running.await.unwrap();
    assert_eq!(s, 200);
    let done = parsed(&b);
    assert_eq!(done["status"], "completed");
    assert_eq!(done["cancel_requested"], true);
    assert_eq!(f.peer.captured.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn changed_category_or_caller_overrides_cannot_reuse_approval() {
    let f = Fixture::new().await;
    let (id, a) = f.proposal().await;
    f.approve(&id, &a).await;
    let (s, _) = call(
        &f.app,
        "POST",
        &Fixture::path(&id, &a, "dispatch"),
        &f.key,
        Some(json!({"title":"Changed"})),
    )
    .await;
    assert_eq!(s, 422);
    *f.peer.category_name.lock().unwrap() = "Changed category".into();
    let (s, _) = call(
        &f.app,
        "POST",
        &Fixture::path(&id, &a, "dispatch"),
        &f.key,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 409);
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn simulation_and_model_chosen_operation_ids_are_refused() {
    let f = Fixture::new().await;
    let id = f.conversation().await;
    let (s, _) = call(
        &f.app,
        "POST",
        &format!("/v1/conversations/{id}/turns"),
        &f.key,
        Some(f.turn("simulation")),
    )
    .await;
    assert_eq!(s, 422);
    assert!(f.model.received_requests().await.unwrap().is_empty());
    f.peer.bad_proposal.store(true, Ordering::SeqCst);
    let (s, b) = call(
        &f.app,
        "POST",
        &format!("/v1/conversations/{id}/turns"),
        &f.key,
        Some(f.turn("real_world")),
    )
    .await;
    assert_eq!(s, 200);
    assert!(String::from_utf8(b)
        .unwrap()
        .contains("invalid memory arguments"));
    let (s, b) = call(
        &f.app,
        "GET",
        &format!("/v1/conversations/{id}/actions"),
        &f.key,
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(parsed(&b), json!([]));
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_changed_stored_payload_cannot_reuse_its_old_approval_digest() {
    let f = Fixture::new().await;
    let (id, a) = f.proposal().await;
    f.approve(&id, &a).await;
    sqlx::query("UPDATE gateway_actions SET payload=jsonb_set(payload,'{input,title}','\"Changed after approval\"') WHERE id=$1")
        .bind(Uuid::parse_str(a["id"].as_str().unwrap()).unwrap()).execute(f.state.store().pool()).await.unwrap();
    let (s, _) = call(
        &f.app,
        "POST",
        &Fixture::path(&id, &a, "dispatch"),
        &f.key,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 409);
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn explicit_retry_retains_the_original_operation_and_creates_no_duplicate() {
    let f = Fixture::new().await;
    let (id, a) = f.proposal().await;
    f.approve(&id, &a).await;
    f.peer.lost_reply.store(true, Ordering::SeqCst);
    assert_eq!(f.command(&id, &a, "dispatch").await["status"], "uncertain");
    let receipt = f
        .peer
        .captured
        .lock()
        .unwrap()
        .get(a["id"].as_str().unwrap())
        .unwrap()
        .clone();
    let retried = f.command(&id, &a, "retry").await;
    assert_eq!(retried["status"], "completed");
    assert_eq!(retried["receipt"], receipt);
    assert_eq!(retried["attempts"], 2);
    assert_eq!(f.peer.captured.lock().unwrap().len(), 1);
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn stopped_dispatch_future_leaves_a_durable_reconcilable_operation() {
    let f = Fixture::new().await;
    let (id, a) = f.proposal().await;
    f.approve(&id, &a).await;
    f.peer.slow_capture.store(true, Ordering::SeqCst);
    let app = f.app.clone();
    let key = f.key.clone();
    let path = Fixture::path(&id, &a, "dispatch");
    let running =
        tokio::spawn(async move { call(&app, "POST", &path, &key, Some(json!({}))).await });
    until(|| f.peer.captures.load(Ordering::SeqCst) == 1).await;
    running.abort();
    let _ = running.await;
    let restarted = build_router(f.state.clone());
    let mut done = None;
    for _ in 0..50 {
        let (s, b) = call(
            &restarted,
            "POST",
            &Fixture::path(&id, &a, "reconcile"),
            &f.key,
            Some(json!({})),
        )
        .await;
        if s == 200 {
            done = Some(parsed(&b));
            break;
        }
        assert_eq!(s, 409);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(done.unwrap()["status"], "completed");
    assert_eq!(f.peer.captured.lock().unwrap().len(), 1);
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn canceling_an_uncertain_operation_never_retries_a_write() {
    let f = Fixture::new().await;
    let (id, a) = f.proposal().await;
    f.approve(&id, &a).await;
    f.peer.lost_reply.store(true, Ordering::SeqCst);
    assert_eq!(f.command(&id, &a, "dispatch").await["status"], "uncertain");
    assert_eq!(f.command(&id, &a, "cancel").await["cancel_requested"], true);
    let (s, _) = call(
        &f.app,
        "POST",
        &Fixture::path(&id, &a, "retry"),
        &f.key,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 409);
    let reconciled = f.command(&id, &a, "reconcile").await;
    assert_eq!(reconciled["status"], "completed");
    assert_eq!(reconciled["cancel_requested"], true);
    assert_eq!(f.peer.captures.load(Ordering::SeqCst), 1);
}
