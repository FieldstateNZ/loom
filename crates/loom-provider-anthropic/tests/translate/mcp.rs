//! The MCP connector: native `mcp_servers` entries and lossless round-tripping
//! of `mcp_tool_use` / `mcp_tool_result` blocks.

use loom_core::{
    ContentPart, Conversation, ConversationOptions, McpServerRef, Message, ProviderBinding, Role,
};
use loom_provider_anthropic::translate;
use serde_json::json;
use uuid::Uuid;

use super::support::bound;

#[test]
fn mcp_servers_translate_to_native_field_with_injected_token() {
    let conversation = bound();
    let mut options = ConversationOptions::new();
    // A resolved reference: URL and token have been injected server-side, plus
    // a tool_configuration forwarded verbatim.
    options.mcp_servers = vec![McpServerRef {
        name: "github".to_owned(),
        url: Some("https://mcp.githubcopilot.com/mcp".to_owned()),
        authorization: Some("mcp-secret-token".to_owned()),
        tool_configuration: Some(json!({ "enabled": true, "allowed_tools": ["search"] })),
    }];

    let request = translate::translate_request(&conversation, &options);
    let servers = request["mcp_servers"]
        .as_array()
        .expect("mcp_servers array");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["type"], json!("url"));
    assert_eq!(servers[0]["name"], json!("github"));
    assert_eq!(
        servers[0]["url"],
        json!("https://mcp.githubcopilot.com/mcp")
    );
    // The token is emitted into the native request body (and only there).
    assert_eq!(servers[0]["authorization_token"], json!("mcp-secret-token"));
    assert_eq!(
        servers[0]["tool_configuration"],
        json!({ "enabled": true, "allowed_tools": ["search"] })
    );

    // Offering an MCP server requires the connector's beta token, deterministic
    // and catalogue-driven.
    assert_eq!(
        translate::required_betas(&conversation, &options),
        vec!["mcp-client-2025-04-04".to_owned()]
    );
}

#[test]
fn mcp_server_without_token_omits_authorization_field() {
    let conversation = bound();
    let mut options = ConversationOptions::new();
    options.mcp_servers = vec![McpServerRef::inline(
        "public",
        "https://mcp.example.com/mcp",
        None,
    )];

    let request = translate::translate_request(&conversation, &options);
    let servers = request["mcp_servers"]
        .as_array()
        .expect("mcp_servers array");
    assert!(servers[0].get("authorization_token").is_none());
    assert!(servers[0].get("tool_configuration").is_none());
}

#[test]
fn mcp_tool_use_and_result_blocks_round_trip_verbatim_via_provider_extension() {
    // MCP connector blocks carry provider-specific fields (`server_name`,
    // `is_error`) that Loom does not model, so they ride through the escape
    // hatch verbatim rather than being reshaped into a typed server-tool part.
    let mcp_use = json!({
        "type": "mcp_tool_use",
        "id": "mcptoolu_1",
        "name": "search_issues",
        "server_name": "github",
        "input": { "query": "loom" }
    });
    let mcp_result = json!({
        "type": "mcp_tool_result",
        "tool_use_id": "mcptoolu_1",
        "is_error": false,
        "content": [{ "type": "text", "text": "found 3 issues" }]
    });

    for block in [&mcp_use, &mcp_result] {
        match translate::block_to_part(block) {
            ContentPart::ProviderExtension {
                provider, payload, ..
            } => {
                assert_eq!(provider, "anthropic");
                // The payload is preserved byte-for-byte, including the fields a
                // typed server-tool part would have dropped.
                assert_eq!(&payload, block);
            }
            other => panic!("expected ProviderExtension, got {other:?}"),
        }
    }

    // And the parts re-emit to the identical native blocks (lossless).
    let mut conversation = Conversation::new(
        Uuid::new_v4(),
        ProviderBinding::new("anthropic", "claude-opus-4-8"),
    );
    conversation.messages.push(Message::new(
        Role::Assistant,
        vec![
            translate::block_to_part(&mcp_use),
            translate::block_to_part(&mcp_result),
        ],
    ));
    let request = translate::translate_request(&conversation, &ConversationOptions::new());
    assert_eq!(request["messages"][0]["content"][0], mcp_use);
    assert_eq!(request["messages"][0]["content"][1], mcp_result);
}

#[test]
fn unknown_mcp_block_shape_becomes_provider_extension_without_error() {
    // A future MCP block Loom does not model must ride through the escape hatch,
    // never error.
    let native = json!({
        "type": "mcp_tool_progress",
        "server_name": "github",
        "detail": { "step": 2 }
    });
    match translate::block_to_part(&native) {
        ContentPart::ProviderExtension { kind, payload, .. } => {
            assert_eq!(kind, "mcp_tool_progress");
            assert_eq!(payload, native);
        }
        other => panic!("expected ProviderExtension, got {other:?}"),
    }
}
