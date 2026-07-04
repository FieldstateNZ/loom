//! Integration tests for multi-tenancy, virtual keys and auth middleware.
//!
//! Each test boots a throwaway PostgreSQL 16 container via `testcontainers`,
//! applies the store migrations, builds the real router, and drives it through
//! `tower::ServiceExt::oneshot` — exercising the full auth stack against a live
//! database.

use axum::body::Body;
use axum::response::Response;
use axum::Router;
use http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use loom_core::{Conversation, Message, ProviderBinding};
use loom_server::{build_router, AppState, Crypto, KeyHasher};
use loom_store::{ConversationStore, CredentialStore, KeyStore, PgStore};

use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

const ADMIN_TOKEN: &str = "root-admin-secret-token";
const ENC_KEY: [u8; 32] = [0x11; 32];
const PEPPER: &[u8] = b"integration-test-pepper";

/// Boots a fresh migrated database and returns the container (kept alive for the
/// test), the connected store, and the assembled router.
async fn setup() -> (ContainerAsync<Postgres>, PgStore, Router) {
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
    let store = PgStore::connect(&url).await.expect("connect to postgres");
    loom_store::run_migrations(store.pool())
        .await
        .expect("migrations apply cleanly");

    let state = AppState::new(
        store.clone(),
        Crypto::new(ENC_KEY),
        KeyHasher::new(PEPPER.to_vec()),
        ADMIN_TOKEN.to_owned(),
    );
    let app = build_router(state);
    (container, store, app)
}

/// Sends a request through a clone of the router and returns the response.
async fn send(app: &Router, req: Request<Body>) -> Response {
    app.clone()
        .oneshot(req)
        .await
        .expect("router handled request")
}

/// Reads a JSON response body.
async fn json_body(resp: Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("body is JSON")
}

/// Builds a JSON request with an optional bearer token.
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

/// Creates a tenant via the admin API and returns its id.
async fn create_tenant(app: &Router, slug: &str, name: &str) -> Uuid {
    let resp = send(
        app,
        request(
            "POST",
            "/admin/tenants",
            Some(ADMIN_TOKEN),
            Some(json!({ "slug": slug, "name": name })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = json_body(resp).await;
    Uuid::parse_str(body["id"].as_str().unwrap()).unwrap()
}

/// Mints a key via the admin API and returns `(key_id, plaintext, prefix)`.
async fn create_key(app: &Router, tenant_id: Uuid, name: &str) -> (Uuid, String, String) {
    let resp = send(
        app,
        request(
            "POST",
            &format!("/admin/tenants/{tenant_id}/keys"),
            Some(ADMIN_TOKEN),
            Some(json!({ "name": name })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = json_body(resp).await;
    (
        Uuid::parse_str(body["id"].as_str().unwrap()).unwrap(),
        body["key"].as_str().unwrap().to_owned(),
        body["key_prefix"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn missing_key_is_unauthorized_with_envelope() {
    let (_pg, _store, app) = setup().await;
    let resp = send(&app, request("GET", "/v1/whoami", None, None)).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body = json_body(resp).await;
    assert_eq!(body["error"]["code"], "unauthorized");
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn invalid_key_is_unauthorized() {
    let (_pg, _store, app) = setup().await;
    let resp = send(
        &app,
        request("GET", "/v1/whoami", Some("loom_live_not-a-real-key"), None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(resp).await["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn valid_key_resolves_tenant_context() {
    let (_pg, _store, app) = setup().await;
    let tenant = create_tenant(&app, "acme", "Acme Inc").await;
    let (key_id, secret, prefix) = create_key(&app, tenant, "primary").await;

    assert!(secret.starts_with("loom_live_"));
    assert!(prefix.starts_with("loom_live_"));

    let resp = send(&app, request("GET", "/v1/whoami", Some(&secret), None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["tenant_id"], tenant.to_string());
    assert_eq!(body["key_id"], key_id.to_string());
    assert_eq!(body["key_prefix"], prefix);
}

#[tokio::test]
async fn same_key_authenticates_repeatedly_via_deterministic_hash() {
    let (_pg, store, app) = setup().await;
    let tenant = create_tenant(&app, "acme", "Acme Inc").await;
    let (_id, secret, _prefix) = create_key(&app, tenant, "primary").await;

    // The plaintext is never stored: a lookup by the raw key finds nothing,
    // but a lookup by its peppered hash finds the row.
    let hasher = KeyHasher::new(PEPPER.to_vec());
    assert!(store.get_key_by_hash(&secret).await.unwrap().is_none());
    assert!(store
        .get_key_by_hash(&hasher.hash(&secret))
        .await
        .unwrap()
        .is_some());

    // Presenting the same key twice both succeed (deterministic hash).
    for _ in 0..2 {
        let resp = send(&app, request("GET", "/v1/whoami", Some(&secret), None)).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn revoked_key_is_immediately_unauthorized() {
    let (_pg, _store, app) = setup().await;
    let tenant = create_tenant(&app, "acme", "Acme Inc").await;
    let (key_id, secret, _prefix) = create_key(&app, tenant, "primary").await;

    // Works before revocation.
    let ok = send(&app, request("GET", "/v1/whoami", Some(&secret), None)).await;
    assert_eq!(ok.status(), StatusCode::OK);

    // Revoke.
    let revoke = send(
        &app,
        request(
            "DELETE",
            &format!("/admin/keys/{key_id}"),
            Some(ADMIN_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

    // Immediately rejected — no cache.
    let after = send(&app, request("GET", "/v1/whoami", Some(&secret), None)).await;
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(after).await["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn admin_endpoints_require_root_token() {
    let (_pg, _store, app) = setup().await;

    // No token.
    let none = send(
        &app,
        request(
            "POST",
            "/admin/tenants",
            None,
            Some(json!({ "slug": "a", "name": "A" })),
        ),
    )
    .await;
    assert_eq!(none.status(), StatusCode::UNAUTHORIZED);

    // Wrong token.
    let wrong = send(
        &app,
        request(
            "POST",
            "/admin/tenants",
            Some("not-the-root-token"),
            Some(json!({ "slug": "a", "name": "A" })),
        ),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(wrong).await["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn cross_tenant_access_is_isolated() {
    let (_pg, store, app) = setup().await;
    let tenant_a = create_tenant(&app, "tenant-a", "Tenant A").await;
    let tenant_b = create_tenant(&app, "tenant-b", "Tenant B").await;
    let (_ka, key_a, _pa) = create_key(&app, tenant_a, "a-key").await;
    let (_kb, key_b, _pb) = create_key(&app, tenant_b, "b-key").await;

    // A conversation owned by tenant A.
    let mut convo = Conversation::new(tenant_a, ProviderBinding::new("mock", "m"));
    convo.messages.push(Message::user("hello from A"));
    store.create_conversation(&convo).await.unwrap();

    // Tenant A's key can read it.
    let a_view = send(
        &app,
        request(
            "GET",
            &format!("/v1/conversations/{}", convo.id),
            Some(&key_a),
            None,
        ),
    )
    .await;
    assert_eq!(a_view.status(), StatusCode::OK);
    assert_eq!(json_body(a_view).await["tenant_id"], tenant_a.to_string());

    // Tenant B's key cannot — the store scopes the lookup, yielding 404.
    let b_view = send(
        &app,
        request(
            "GET",
            &format!("/v1/conversations/{}", convo.id),
            Some(&key_b),
            None,
        ),
    )
    .await;
    assert_eq!(b_view.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(b_view).await["error"]["code"], "not_found");
}

#[tokio::test]
async fn credential_round_trips_through_encryption() {
    let (_pg, store, app) = setup().await;
    let tenant = create_tenant(&app, "acme", "Acme Inc").await;

    let plaintext = "sk-ant-super-secret-value";
    let put = send(
        &app,
        request(
            "PUT",
            &format!("/admin/tenants/{tenant}/credentials/anthropic"),
            Some(ADMIN_TOKEN),
            Some(json!({ "api_key": plaintext, "base_url": "https://example.test" })),
        ),
    )
    .await;
    assert_eq!(put.status(), StatusCode::OK);
    let body = json_body(put).await;
    // The response never echoes the secret.
    assert_eq!(body["provider"], "anthropic");
    assert!(body.get("api_key").is_none());

    // Load the stored row and confirm ciphertext != plaintext, and that it
    // decrypts back to the original under the gateway key.
    let stored = store
        .get_credential(Some(tenant), "anthropic")
        .await
        .unwrap()
        .expect("credential stored");
    assert_ne!(stored.encrypted_secret, plaintext.as_bytes());
    assert_eq!(stored.base_url.as_deref(), Some("https://example.test"));

    let crypto = Crypto::new(ENC_KEY);
    let nonce = stored.nonce.expect("nonce persisted");
    let decrypted = crypto.decrypt(&nonce, &stored.encrypted_secret).unwrap();
    assert_eq!(decrypted, plaintext.as_bytes());
}

#[tokio::test]
async fn readyz_reports_ready() {
    let (_pg, _store, app) = setup().await;
    let resp = send(&app, request("GET", "/readyz", None, None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(json_body(resp).await["status"], "ready");
}

#[tokio::test]
async fn healthz_is_open() {
    let (_pg, _store, app) = setup().await;
    let resp = send(&app, request("GET", "/healthz", None, None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
}
