//! Tests for [`McpServerRef`]'s secret-handling guarantees: its bearer token
//! is redacted from `Debug` and never serialized.

use loom_core::McpServerRef;
use serde_json::json;

#[test]
fn mcp_server_ref_debug_redacts_the_authorization_token() {
    // The bearer token must never appear in a log line, panic message, or test
    // failure — Debug redacts it while still showing the other fields.
    let server = McpServerRef {
        name: "github".to_owned(),
        url: Some("https://mcp.example.com/mcp".to_owned()),
        authorization: Some("super-secret-token".to_owned()),
        tool_configuration: None,
    };
    let debug = format!("{server:?}");
    assert!(
        !debug.contains("super-secret-token"),
        "token must be redacted"
    );
    assert!(debug.contains("<redacted>"));
    assert!(debug.contains("github"));

    // A named reference carries no token and shows None.
    let named = format!("{:?}", McpServerRef::named("github"));
    assert!(named.contains("authorization: None"));
}

#[test]
fn mcp_authorization_token_is_never_serialized() {
    // The bearer token is deserialize-only: an inbound inline reference may
    // carry it, but serializing a McpServerRef must never emit it — so a
    // decrypted token cannot leak through anything that renders options to JSON
    // (e.g. request telemetry).
    let server = McpServerRef {
        name: "inline".to_owned(),
        url: Some("https://mcp.example.com/mcp".to_owned()),
        authorization: Some("super-secret-token".to_owned()),
        tool_configuration: None,
    };
    let value = serde_json::to_value(&server).expect("serialize");
    assert!(
        value.get("authorization").is_none(),
        "authorization must never serialize"
    );
    assert!(!serde_json::to_string(&server)
        .unwrap()
        .contains("super-secret-token"));

    // ...but it still deserializes inbound, so inline callers can supply it.
    let parsed: McpServerRef = serde_json::from_value(json!({
        "name": "inline",
        "url": "https://mcp.example.com/mcp",
        "authorization": "tok",
    }))
    .expect("deserialize");
    assert_eq!(parsed.authorization.as_deref(), Some("tok"));
}
