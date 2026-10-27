//! Real database/router qualification for atomic, recoverable initial setup.
use axum::{
    body::{to_bytes, Body},
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use http::{Request, StatusCode};
use loom_server::{build_router, AppState, Crypto, KeyHasher};
use loom_store::{run_migrations, verify_migrations, PgStore};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "private-account-synthetic-admin";
fn router(store: PgStore) -> Router {
    build_router(AppState::new(
        store,
        Crypto::new([17; 32]),
        KeyHasher::new(b"private-account-synthetic-pepper".to_vec()),
        ADMIN.to_owned(),
    ))
}
fn input(subject: &str, n: u8) -> (String, Value) {
    let issuer = "https://cognito-idp.ap-southeast-2.amazonaws.com/ap-southeast-2_Synthetic";
    let owner = serde_json::to_vec(&[issuer, subject]).unwrap();
    let namespace = format!("tandem-{}", &hex::encode(Sha256::digest(owner))[..40]);
    (
        namespace,
        json!({"schema_version":1,"issuer":issuer,"subject":subject,"operation_id":Uuid::new_v4(),"key":format!("loom_test_{}",URL_SAFE_NO_PAD.encode([n;32]))}),
    )
}
async fn call(
    app: &Router,
    namespace: &str,
    value: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("PUT")
        .uri(format!("/admin/private-accounts/{namespace}"))
        .header("Content-Type", "application/json");
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(
            builder
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    if status.is_success() {
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    let body = to_bytes(response.into_body(), 16384).await.unwrap();
    assert!(!String::from_utf8_lossy(&body)
        .contains(value["key"].as_str().unwrap_or("no-key-sentinel")));
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn initial_account_replays_after_router_restart_and_authenticates_original_key() {
    let pg = Postgres::default().with_tag("16").start().await.unwrap();
    let store = PgStore::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        pg.get_host_port_ipv4(5432).await.unwrap()
    ))
    .await
    .unwrap();
    run_migrations(store.pool()).await.unwrap();
    let app = router(store.clone());
    let (ns, request) = input("opaque-CaSe-sensitive-sub", 31);
    let (status, first) = call(&app, &ns, request.clone(), Some(ADMIN)).await;
    assert_eq!(status, StatusCode::CREATED);
    let restarted = router(store.clone());
    let (status, replay) = call(&restarted, &ns, request.clone(), Some(ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay, first);
    let response = restarted
        .oneshot(
            Request::builder()
                .uri("/v1/whoami")
                .header(
                    "Authorization",
                    format!("Bearer {}", request["key"].as_str().unwrap()),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let who: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    assert_eq!(who["tenant_id"], first["tenant_id"]);
    assert_eq!(who["key_id"], first["key_id"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM private_accounts")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM virtual_keys")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn concurrent_replicas_commit_one_initial_identity_and_never_rotate_it() {
    let pg = Postgres::default().with_tag("16").start().await.unwrap();
    let store = PgStore::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        pg.get_host_port_ipv4(5432).await.unwrap()
    ))
    .await
    .unwrap();
    run_migrations(store.pool()).await.unwrap();
    let a = router(store.clone());
    let b = router(store.clone());
    let (ns, request) = input("concurrent-owner", 32);
    let results = futures::future::join_all((0..12).map(|i| {
        call(
            if i % 2 == 0 { &a } else { &b },
            &ns,
            request.clone(),
            Some(ADMIN),
        )
    }))
    .await;
    assert_eq!(
        results
            .iter()
            .filter(|(s, _)| *s == StatusCode::CREATED)
            .count(),
        1
    );
    for (status, body) in &results {
        assert!(*status == StatusCode::CREATED || *status == StatusCode::OK);
        assert_eq!(body, &results[0].1);
    }
    let mut changed = request.clone();
    changed["operation_id"] = json!(Uuid::new_v4());
    assert_eq!(
        call(&a, &ns, changed, Some(ADMIN)).await.0,
        StatusCode::CONFLICT
    );
    changed = request.clone();
    changed["key"] = input("other", 33).1["key"].clone();
    assert_eq!(
        call(&a, &ns, changed, Some(ADMIN)).await.0,
        StatusCode::CONFLICT
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tenants")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn rejects_unverified_route_inputs_and_never_adopts_existing_or_revoked_resources() {
    let pg = Postgres::default().with_tag("16").start().await.unwrap();
    let store = PgStore::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        pg.get_host_port_ipv4(5432).await.unwrap()
    ))
    .await
    .unwrap();
    run_migrations(store.pool()).await.unwrap();
    let app = router(store.clone());
    let (ns, request) = input("owner", 34);
    assert_eq!(
        call(&app, &ns, request.clone(), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            &ns,
            request.clone(),
            Some(request["key"].as_str().unwrap())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let cases = [
        json!({"schema_version":2}),
        json!({"operation_id":Uuid::nil()}),
        json!({"key":"loom_test_short"}),
        json!({"subject":"different-owner"}),
        json!({"issuer":"https://example.invalid/?secret=x"}),
        json!({"unexpected":"field"}),
    ];
    for change in cases {
        let mut invalid = request.clone();
        for (k, v) in change.as_object().unwrap() {
            invalid[k] = v.clone();
        }
        assert!(call(&app, &ns, invalid, Some(ADMIN))
            .await
            .0
            .is_client_error());
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tenants")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        0
    );
    let first = call(&app, &ns, request.clone(), Some(ADMIN)).await.1;
    let (different_owner, mut reused_key) = input("key-reuse-owner", 41);
    reused_key["key"] = request["key"].clone();
    assert_eq!(
        call(&app, &different_owner, reused_key, Some(ADMIN))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut oversized = request.clone();
    oversized["key"] = json!("x".repeat(9000));
    let (status, rejected) = call(&app, &ns, oversized, Some(ADMIN)).await;
    // The shared envelope maps a buffering limit to 400. This fixed message
    // proves rejection occurred in the bounded reader, before key validation.
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        rejected["error"]["message"],
        "request body could not be read"
    );
    sqlx::query("UPDATE virtual_keys SET status='revoked' WHERE id=$1")
        .bind(Uuid::parse_str(first["key_id"].as_str().unwrap()).unwrap())
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(
        call(&app, &ns, request, Some(ADMIN)).await.0,
        StatusCode::CONFLICT
    );
    let (other, mut other_request) = input("second-owner", 35);
    other_request["operation_id"] = first["operation_id"].clone();
    assert_eq!(
        call(&app, &other, other_request, Some(ADMIN)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tenants")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        1,
        "failed receipt must roll back tenant and key"
    );
    let (preexisting, req) = input("preexisting-owner", 36);
    sqlx::query("INSERT INTO tenants(id,slug,name) VALUES($1,$2,'legacy')")
        .bind(Uuid::new_v4())
        .bind(&preexisting)
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(
        call(&app, &preexisting, req, Some(ADMIN)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM virtual_keys")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn upgrades_previous_release_and_replay_does_not_reactivate_a_suspended_tenant() {
    let pg = Postgres::default().with_tag("16").start().await.unwrap();
    let store = PgStore::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        pg.get_host_port_ipv4(5432).await.unwrap()
    ))
    .await
    .unwrap();
    let mut previous = sqlx::migrate!("../loom-store/migrations");
    previous.migrations = std::borrow::Cow::Owned(
        previous
            .iter()
            .filter(|m| m.version < 20261029000000)
            .cloned()
            .collect(),
    );
    previous.run(store.pool()).await.unwrap();
    assert!(verify_migrations(store.pool()).await.is_err());
    run_migrations(store.pool()).await.unwrap();
    run_migrations(store.pool()).await.unwrap();
    verify_migrations(store.pool()).await.unwrap();
    let app = router(store.clone());
    let (ns, request) = input("paused-owner", 37);
    let first = call(&app, &ns, request.clone(), Some(ADMIN)).await.1;
    sqlx::query("UPDATE tenants SET status='suspended' WHERE id=$1")
        .bind(Uuid::parse_str(first["tenant_id"].as_str().unwrap()).unwrap())
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(
        call(&app, &ns, request, Some(ADMIN)).await.0,
        StatusCode::CONFLICT
    );
}
