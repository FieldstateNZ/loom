//! Serde round-trip tests for [`ConversationOptions`] and [`ServerTool`].

mod common;

use common::assert_json_roundtrip;
use loom_core::{
    CacheHint, CacheNegotiation, CacheTtl, ConversationOptions, McpServerRef, ServerTool,
    ToolDefinition,
};
use serde_json::json;

#[test]
fn conversation_options_roundtrips() {
    let mut options = ConversationOptions::new();
    options.temperature = Some(0.7);
    options.max_tokens = Some(1024);
    options.stop_sequences = vec!["STOP".into()];
    options.tools = vec![ToolDefinition {
        name: "get_weather".into(),
        description: Some("Look up the weather".into()),
        input_schema: json!({ "type": "object", "properties": {} }),
        cache: Some(CacheHint::with_ttl(CacheTtl::OneHour)),
    }];
    options.auto_cache = true;
    options.cache_negotiation = CacheNegotiation::HardFail;
    options.server_tools = vec![
        ServerTool::WebSearch {
            max_uses: Some(5),
            allowed_domains: Some(vec!["example.com".into()]),
            blocked_domains: None,
        },
        ServerTool::CodeExecution {},
        // The Raw passthrough carries a native tool definition verbatim.
        ServerTool::Raw(json!({ "type": "web_search_20250305", "name": "web_search" })),
    ];
    options.mcp_servers = vec![
        McpServerRef::named("github"),
        McpServerRef {
            name: "inline".to_owned(),
            url: Some("https://mcp.example.com/mcp".to_owned()),
            // `authorization` is deserialize-only (never serialized), so it is
            // deliberately absent here — the round-trip helper asserts equality,
            // and a serialized token would not survive by design. Its
            // never-serialized guarantee is covered by the dedicated test below.
            authorization: None,
            tool_configuration: Some(json!({ "enabled": true })),
        },
    ];
    options.provider_options.insert(
        "anthropic".to_owned(),
        json!({ "tool_choice": { "type": "auto" }, "top_p": 0.9 }),
    );
    assert_json_roundtrip(&options);
    assert_json_roundtrip(&ConversationOptions::new());
}

#[test]
fn server_tool_variants_roundtrip_and_are_kind_tagged() {
    let tools = vec![
        ServerTool::WebSearch {
            max_uses: None,
            allowed_domains: None,
            blocked_domains: None,
        },
        ServerTool::WebSearch {
            max_uses: Some(3),
            allowed_domains: None,
            blocked_domains: Some(vec!["spam.example".into()]),
        },
        ServerTool::CodeExecution {},
        ServerTool::Raw(json!({ "type": "code_execution_20250522", "name": "code_execution" })),
    ];
    for tool in &tools {
        assert_json_roundtrip(tool);
    }

    // The discriminator is a Loom-owned `kind`, distinct from any provider-native
    // `type` a Raw payload carries.
    assert_eq!(
        serde_json::to_value(ServerTool::CodeExecution {}).unwrap(),
        json!({ "kind": "code_execution" })
    );
    let raw = serde_json::to_value(ServerTool::Raw(
        json!({ "type": "web_search_20250305", "name": "web_search" }),
    ))
    .unwrap();
    assert_eq!(raw["kind"], json!("raw"));
    assert_eq!(raw["type"], json!("web_search_20250305"));
}
