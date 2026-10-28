//! Actual SDK HTTP bytes checked with independent test-only canonicalization.
use super::*;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

const ACCESS: &str = "ASIASYNTHETICEXAMPLE";
const SECRET: &str = "synthetic-role-secret-never-used-on-AWS";
const SESSION: &str = "synthetic-role-session-never-used-on-AWS";
const BEARER: &str = "Bearer synthetic-private-memory-token";
fn client(endpoint: &str) -> PrivateHttpClient {
    let mut client = PrivateHttpClient::new(&Url::parse(endpoint).unwrap()).unwrap();
    client.signing = Some(SigningTarget {
        region: "ap-southeast-2".into(),
        credentials: Arc::new(|| {
            Ok(Credentials::new(
                ACCESS,
                SECRET,
                Some(SESSION.into()),
                None,
                "synthetic",
            ))
        }),
    });
    client
}
fn binding_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    let mut auth = HeaderValue::from_static(BEARER);
    auth.set_sensitive(true);
    headers.insert("authorization", auth);
    for (name, id) in [
        (
            "x-lucidbrain-expected-principal",
            "11111111-1111-4111-8111-111111111111",
        ),
        (
            "x-lucidbrain-expected-workspace",
            "22222222-2222-4222-8222-222222222222",
        ),
        (
            "x-lucidbrain-expected-agent",
            "33333333-3333-4333-8333-333333333333",
        ),
        (
            "x-lucidbrain-expected-credential",
            "44444444-4444-4444-8444-444444444444",
        ),
    ] {
        headers.insert(name, HeaderValue::from_static(id));
    }
    headers
}
fn digest(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(bytes.as_ref()))
}
fn hmac(key: impl AsRef<[u8]>, bytes: impl AsRef<[u8]>) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_ref()).unwrap();
    mac.update(bytes.as_ref());
    mac.finalize().into_bytes().to_vec()
}
fn verify(request: &wiremock::Request) -> bool {
    let verify = || -> Option<bool> {
        let headers = &request.headers;
        let auth = headers.get("authorization")?.to_str().ok()?;
        let tail = auth.strip_prefix("AWS4-HMAC-SHA256 Credential=")?;
        let (credential, tail) = tail.split_once(", SignedHeaders=")?;
        let (names, signature) = tail.split_once(", Signature=")?;
        let credential: Vec<_> = credential.split('/').collect();
        if credential.len() != 5
            || credential[0] != ACCESS
            || credential[2] != "ap-southeast-2"
            || credential[3] != "lambda"
            || credential[4] != "aws4_request"
        {
            return Some(false);
        }
        let date = headers.get("x-amz-date")?.to_str().ok()?;
        if date.get(..8)? != credential[1]
            || headers.get("x-amz-security-token")?.to_str().ok()? != SESSION
            || headers.get("x-amz-content-sha256")?.to_str().ok()? != digest(&request.body)
        {
            return Some(false);
        }
        let ordered: Vec<_> = names.split(';').collect();
        let mut sorted = ordered.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted != ordered {
            return Some(false);
        }
        for name in [
            "host",
            "x-amz-date",
            "x-amz-security-token",
            "x-amz-content-sha256",
        ] {
            if !ordered.contains(&name) {
                return Some(false);
            }
        }
        let mut canonical_headers = String::new();
        for name in ordered {
            canonical_headers.push_str(&format!(
                "{name}:{}\n",
                headers
                    .get(name)?
                    .to_str()
                    .ok()?
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        let canonical = format!(
            "{}\n{}\n\n{}\n{}\n{}",
            request.method,
            request.url.path(),
            canonical_headers,
            names,
            digest(&request.body)
        );
        let scope = credential[1..].join("/");
        let value = format!("AWS4-HMAC-SHA256\n{date}\n{scope}\n{}", digest(canonical));
        let key = hmac(
            hmac(
                hmac(
                    hmac(format!("AWS4{SECRET}"), credential[1]),
                    "ap-southeast-2",
                ),
                "lambda",
            ),
            "aws4_request",
        );
        Some(hex::encode(hmac(key, value)) == signature)
    };
    verify() == Some(true)
}

#[tokio::test]
async fn context_and_recoverable_renewal_sign_exact_body_binding_and_current_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/mcp/context"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .mount(&server)
        .await;
    let mut http = client(&(server.uri() + "/mcp"));
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    http.signing.as_mut().unwrap().credentials = Arc::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(Credentials::new(
            ACCESS,
            SECRET,
            Some(SESSION.into()),
            None,
            "synthetic",
        ))
    });
    http.get(Url::parse(&(server.uri() + "/mcp/context")).unwrap())
        .headers(binding_headers())
        .send()
        .await
        .unwrap();
    let pending = json!({"grant_type":"recoverable","operation_id":"same-owned-operation","refresh_token":"synthetic-original-pending-token","note":"Riley 🌏"});
    http.post(Url::parse(&(server.uri() + "/api/v1/oauth/token")).unwrap())
        .json(&pending)
        .send()
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    for request in &requests {
        assert!(verify(request));
        assert!(!request.headers["authorization"]
            .to_str()
            .unwrap()
            .contains(BEARER));
    }
    assert_eq!(requests[0].headers["x-tandem-authorization"], BEARER);
    for (name, value) in binding_headers() {
        if let Some(name) = name {
            if name != "authorization" {
                assert_eq!(requests[0].headers[&name], value);
            }
        }
    }
    assert!(!requests[1].headers.contains_key("x-tandem-authorization"));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&requests[1].body).unwrap(),
        pending
    );
    let mut changed = requests[0].clone();
    changed.headers.insert(
        "x-lucidbrain-expected-workspace",
        HeaderValue::from_static("changed"),
    );
    assert!(!verify(&changed));
    let mut changed = requests[1].clone();
    changed.body = b"{}".to_vec();
    assert!(!verify(&changed));
}

#[tokio::test]
async fn official_mcp_parser_and_serialization_are_delegated_without_losing_signed_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":{"tools":[]}})),
        )
        .mount(&server)
        .await;
    let http = client(&(server.uri() + "/mcp"));
    let message: ClientJsonRpcMessage =
        serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap();
    let mut headers = binding_headers();
    headers.insert(
        "mcp-protocol-version",
        HeaderValue::from_static("2025-03-26"),
    );
    let response = http
        .post_message_with_max_sse_event_size(
            Arc::from(server.uri() + "/mcp"),
            message,
            None,
            None,
            headers
                .into_iter()
                .filter_map(|(n, v)| n.map(|n| (n, v)))
                .collect(),
            16384,
        )
        .await
        .unwrap();
    assert!(matches!(response, StreamableHttpPostResponse::Json(..)));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(verify(&requests[0]));
    assert_eq!(requests[0].headers["x-tandem-authorization"], BEARER);
    assert_eq!(requests[0].headers["mcp-protocol-version"], "2025-03-26");
    assert_eq!(
        requests[0].headers["accept"],
        "text/event-stream, application/json"
    );
}

#[tokio::test]
async fn sdk_get_delete_and_errors_keep_original_session_semantics_without_retries() {
    let server = MockServer::start().await;
    for verb in ["GET", "DELETE"] {
        Mock::given(method(verb))
            .and(path("/mcp"))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(503).set_body_string("unavailable"))
        .mount(&server)
        .await;
    let http = client(&(server.uri() + "/mcp"));
    let headers = || {
        binding_headers()
            .into_iter()
            .filter_map(|(n, v)| n.map(|n| (n, v)))
            .collect()
    };
    let get = http
        .get_stream_with_max_sse_event_size(
            Arc::from(server.uri() + "/mcp"),
            Some(Arc::from("legacy-session")),
            Some("last-event".into()),
            None,
            headers(),
            8192,
        )
        .await;
    assert!(matches!(
        get,
        Err(StreamableHttpError::ServerDoesNotSupportSse)
    ));
    http.delete_session(
        Arc::from(server.uri() + "/mcp"),
        Arc::from("legacy-session"),
        None,
        headers(),
    )
    .await
    .unwrap();
    let message =
        serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap();
    assert!(http
        .post_message(
            Arc::from(server.uri() + "/mcp"),
            message,
            None,
            None,
            headers()
        )
        .await
        .is_err());
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    for request in &requests {
        assert!(verify(request));
    }
    assert_eq!(requests[0].headers["mcp-session-id"], "legacy-session");
    assert_eq!(requests[0].headers["last-event-id"], "last-event");
    assert_eq!(requests[1].headers["mcp-session-id"], "legacy-session");
}

#[tokio::test]
async fn no_redirect_fallback_foreign_origin_or_oversized_payload_can_forward_auth() {
    let server = MockServer::start().await;
    let other = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", other.uri()))
        .mount(&server)
        .await;
    let http = client(&(server.uri() + "/mcp"));
    let response = http
        .get(Url::parse(&(server.uri() + "/mcp/context")).unwrap())
        .headers(binding_headers())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 302);
    assert!(http
        .get(Url::parse(&(other.uri() + "/mcp/context")).unwrap())
        .headers(binding_headers())
        .send()
        .await
        .is_err());
    assert!(http
        .post(Url::parse(&(server.uri() + "/api/v1/oauth/token")).unwrap())
        .json(&"x".repeat(65537))
        .send()
        .await
        .is_err());
    assert!(http
        .get(Url::parse(&(server.uri() + "/mcp/context?next=attacker")).unwrap())
        .headers(binding_headers())
        .send()
        .await
        .is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert!(other.received_requests().await.unwrap().is_empty());
}

#[test]
fn signer_trace_and_debug_cannot_expose_application_bearer_or_refresh_payload() {
    #[derive(Clone)]
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .without_time()
        .with_ansi(false)
        .with_writer(move || Writer(output.clone()))
        .finish();
    let http = client("http://127.0.0.1:12345/mcp");
    let headers = tracing::subscriber::with_default(subscriber, || {
        http.signed_headers(
            "GET",
            &Url::parse("http://127.0.0.1:12345/mcp/context").unwrap(),
            &binding_headers(),
            &[],
        )
    })
    .unwrap();
    let output = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(!output.contains(BEARER));
    assert!(!output.contains(SESSION));
    assert!(!output.contains(SECRET));
    let debug = format!("{headers:?}");
    assert!(!debug.contains(BEARER));
    assert!(!debug.contains(SESSION));
}
