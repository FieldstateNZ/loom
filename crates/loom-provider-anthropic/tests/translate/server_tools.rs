//! Server (provider-executed) tools: native versioned tool entries and the
//! `anthropic-beta` tokens they require.

use loom_core::{ContentPart, ConversationOptions, ServerTool, ToolDefinition};
use loom_provider_anthropic::translate;
use serde_json::json;

use super::support::bound;

#[test]
fn server_tools_translate_to_native_versioned_tool_entries() {
    let conversation = bound();
    let mut options = ConversationOptions::new();
    // A client tool alongside server tools — both share the native `tools` array.
    options.tools.push(ToolDefinition {
        name: "get_weather".to_owned(),
        description: None,
        input_schema: json!({ "type": "object" }),
        cache: None,
    });
    options.server_tools = vec![
        ServerTool::WebSearch {
            max_uses: Some(4),
            allowed_domains: Some(vec!["example.com".to_owned()]),
            blocked_domains: None,
        },
        ServerTool::CodeExecution {},
        // A native definition Loom does not model — forwarded verbatim.
        ServerTool::Raw(json!({ "type": "web_fetch_20250910", "name": "web_fetch" })),
    ];

    let request = translate::translate_request(&conversation, &options);
    let tools = request["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 4, "client tool + three server tools");

    // Client tool first, then server tools in order.
    assert_eq!(tools[0]["name"], json!("get_weather"));

    // Web search: native versioned type + name + passthrough of the knobs.
    assert_eq!(tools[1]["type"], json!("web_search_20250305"));
    assert_eq!(tools[1]["name"], json!("web_search"));
    assert_eq!(tools[1]["max_uses"], json!(4));
    assert_eq!(tools[1]["allowed_domains"], json!(["example.com"]));
    assert!(tools[1].get("blocked_domains").is_none());

    // Code execution: native versioned type + name, no knobs.
    assert_eq!(tools[2]["type"], json!("code_execution_20250522"));
    assert_eq!(tools[2]["name"], json!("code_execution"));

    // Raw: forwarded byte-for-byte.
    assert_eq!(
        tools[3],
        json!({ "type": "web_fetch_20250910", "name": "web_fetch" })
    );
}

#[test]
fn web_search_only_request_still_emits_the_tools_array() {
    let conversation = bound();
    let mut options = ConversationOptions::new();
    options.server_tools = vec![ServerTool::WebSearch {
        max_uses: None,
        allowed_domains: None,
        blocked_domains: None,
    }];

    let request = translate::translate_request(&conversation, &options);
    let tools = request["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], json!("web_search_20250305"));
    // A bare web-search tool carries only type + name.
    assert!(tools[0].get("max_uses").is_none());
}

#[test]
fn required_betas_are_catalogue_driven_and_caller_overridable() {
    let conversation = bound();

    // Web search is GA — no beta token.
    let mut web = ConversationOptions::new();
    web.server_tools = vec![ServerTool::WebSearch {
        max_uses: None,
        allowed_domains: None,
        blocked_domains: None,
    }];
    assert!(translate::required_betas(&conversation, &web).is_empty());

    // Code execution requires its catalogue-driven beta token.
    let mut code = ConversationOptions::new();
    code.server_tools = vec![ServerTool::CodeExecution {}];
    assert_eq!(
        translate::required_betas(&conversation, &code),
        vec!["code-execution-2025-05-22".to_owned()]
    );

    // A caller can add betas per request (the "no Loom release" path); the set
    // is deterministic and de-duplicated.
    code.provider_options.insert(
        "anthropic".to_owned(),
        json!({ "betas": ["code-execution-2025-05-22", "future-feature-2027-01-01"] }),
    );
    assert_eq!(
        translate::required_betas(&conversation, &code),
        vec![
            "code-execution-2025-05-22".to_owned(),
            "future-feature-2027-01-01".to_owned(),
        ]
    );
}

#[test]
fn reserved_betas_key_never_leaks_into_the_request_body() {
    let conversation = bound();
    let mut options = ConversationOptions::new();
    options.provider_options.insert(
        "anthropic".to_owned(),
        json!({ "betas": ["future-feature-2027-01-01"], "top_p": 0.5 }),
    );

    let request = translate::translate_request(&conversation, &options);
    // The header directive is stripped from the body; other native knobs merge.
    assert!(request.get("betas").is_none());
    assert_eq!(request["top_p"], json!(0.5));
}

#[test]
fn unknown_server_tool_block_becomes_provider_extension_without_error() {
    // A server-tool result shape Loom does not model — neither `server_tool_use`
    // nor a `<name>_tool_result` — must ride through the escape hatch, never
    // error. This is the whole forward-compat point of issue #12.
    let native = json!({
        "type": "code_interpreter_output",
        "id": "ci_1",
        "payload": { "stdout": "42" }
    });
    match translate::block_to_part(&native) {
        ContentPart::ProviderExtension {
            provider,
            kind,
            payload,
        } => {
            assert_eq!(provider, "anthropic");
            assert_eq!(kind, "code_interpreter_output");
            assert_eq!(payload, native);
        }
        other => panic!("expected ProviderExtension, got {other:?}"),
    }
}
