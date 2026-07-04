//! Fixture-based lossless round-trip test.
//!
//! This proves the domain model can represent a realistic Anthropic Messages
//! API response — text, a client `tool_use`, a `thinking` block, a
//! `server_tool_use` + `web_search_tool_result` pair, and an unmodelled
//! `mcp_tool_use` block — losslessly. It maps the fixture's native content
//! blocks **into** [`ContentPart`]s and back **out** to the original Anthropic
//! JSON shape, asserting no semantic loss.
//!
//! The mapping here is a small hand-rolled stand-in for the real provider
//! translation (which lands in issue #4). The point is the domain model, not
//! the mapping: anything not modelled natively (here, `mcp_tool_use`) rides
//! through [`ContentPart::ProviderExtension`] verbatim.

use std::collections::BTreeMap;

use loom_core::{Citation, ContentPart, Usage};
use serde_json::{json, Map, Value};

/// Maps one native Anthropic content block into a [`ContentPart`].
fn block_to_part(block: &Value) -> ContentPart {
    match block["type"].as_str().expect("block type") {
        "text" => ContentPart::Text {
            text: block["text"].as_str().unwrap().to_owned(),
            citations: block.get("citations").map(|c| {
                c.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| Citation(v.clone()))
                    .collect()
            }),
        },
        "thinking" => ContentPart::Thinking {
            thinking: block["thinking"].as_str().unwrap().to_owned(),
            signature: block
                .get("signature")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        "redacted_thinking" => ContentPart::RedactedThinking {
            data: block["data"].as_str().unwrap().to_owned(),
        },
        "tool_use" => ContentPart::ToolUse {
            id: block["id"].as_str().unwrap().to_owned(),
            name: block["name"].as_str().unwrap().to_owned(),
            input: block["input"].clone(),
        },
        "server_tool_use" => ContentPart::ServerToolUse {
            id: block["id"].as_str().unwrap().to_owned(),
            name: block["name"].as_str().unwrap().to_owned(),
            input: block["input"].clone(),
        },
        // Provider-executed tool results share a `<tool>_tool_result` naming
        // convention; the result content is preserved verbatim.
        t if t.ends_with("_tool_result") => ContentPart::ServerToolResult {
            tool_use_id: block["tool_use_id"].as_str().unwrap().to_owned(),
            content: block["content"].clone(),
        },
        // Anything not modelled natively is preserved whole via the escape
        // hatch — no provider feature is ever dropped.
        other => ContentPart::ProviderExtension {
            provider: "anthropic".to_owned(),
            kind: other.to_owned(),
            payload: block.clone(),
        },
    }
}

/// Maps a [`ContentPart`] back out to the native Anthropic block shape.
///
/// `server_tool_names` maps a server-tool `tool_use_id` to the tool's name, so
/// a [`ContentPart::ServerToolResult`] can reconstruct its native block type
/// (e.g. `web_search` → `web_search_tool_result`) — exactly what the real
/// translator does by correlating the result with its originating call.
fn part_to_block(part: &ContentPart, server_tool_names: &BTreeMap<String, String>) -> Value {
    match part {
        ContentPart::Text { text, citations } => {
            let mut obj = Map::new();
            obj.insert("type".into(), json!("text"));
            obj.insert("text".into(), json!(text));
            if let Some(citations) = citations {
                obj.insert(
                    "citations".into(),
                    Value::Array(citations.iter().map(|c| c.0.clone()).collect()),
                );
            }
            Value::Object(obj)
        }
        ContentPart::Thinking {
            thinking,
            signature,
        } => {
            let mut obj = Map::new();
            obj.insert("type".into(), json!("thinking"));
            obj.insert("thinking".into(), json!(thinking));
            if let Some(signature) = signature {
                obj.insert("signature".into(), json!(signature));
            }
            Value::Object(obj)
        }
        ContentPart::RedactedThinking { data } => {
            json!({ "type": "redacted_thinking", "data": data })
        }
        ContentPart::ToolUse { id, name, input } => {
            json!({ "type": "tool_use", "id": id, "name": name, "input": input })
        }
        ContentPart::ServerToolUse { id, name, input } => {
            json!({ "type": "server_tool_use", "id": id, "name": name, "input": input })
        }
        ContentPart::ServerToolResult {
            tool_use_id,
            content,
        } => {
            let name = server_tool_names
                .get(tool_use_id)
                .expect("server tool name for result");
            json!({
                "type": format!("{name}_tool_result"),
                "tool_use_id": tool_use_id,
                "content": content,
            })
        }
        ContentPart::ProviderExtension { payload, .. } => payload.clone(),
        other => panic!("unexpected content part: {other:?}"),
    }
}

/// Maps the fixture's native usage payload into [`Usage`].
fn usage_from_native(native: &Value) -> Usage {
    let server_tool_use = native
        .get("server_tool_use")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default();
    let mut usage = Usage::new();
    usage.input_tokens = native["input_tokens"].as_u64();
    usage.output_tokens = native["output_tokens"].as_u64();
    usage.cache_read_tokens = native["cache_read_input_tokens"].as_u64();
    usage.cache_write_tokens = native["cache_creation_input_tokens"].as_u64();
    usage.server_tool_use = server_tool_use;
    // Preserving the raw payload guarantees a byte-equivalent replay.
    usage.raw = Some(native.clone());
    usage
}

#[test]
fn anthropic_fixture_maps_losslessly() {
    let raw = include_str!("fixtures/anthropic_message.json");
    let fixture: Value = serde_json::from_str(raw).expect("valid fixture JSON");

    let original_blocks = fixture["content"].as_array().expect("content array");

    // Map every native block into the domain model.
    let parts: Vec<ContentPart> = original_blocks.iter().map(block_to_part).collect();

    // Sanity: the native shapes landed on the expected variants, including the
    // escape hatch for the unmodelled `mcp_tool_use` block.
    assert!(matches!(parts[0], ContentPart::Thinking { .. }));
    assert!(matches!(parts[1], ContentPart::Text { .. }));
    assert!(matches!(parts[2], ContentPart::ServerToolUse { .. }));
    assert!(matches!(parts[3], ContentPart::ServerToolResult { .. }));
    assert!(matches!(parts[4], ContentPart::ToolUse { .. }));
    match &parts[5] {
        ContentPart::ProviderExtension {
            provider,
            kind,
            payload,
        } => {
            assert_eq!(provider, "anthropic");
            assert_eq!(kind, "mcp_tool_use");
            assert_eq!(payload["server_name"], json!("github"));
        }
        other => panic!("expected ProviderExtension, got {other:?}"),
    }

    // Correlate server-tool results with their originating calls, as the real
    // translator would.
    let mut server_tool_names = BTreeMap::new();
    for part in &parts {
        if let ContentPart::ServerToolUse { id, name, .. } = part {
            server_tool_names.insert(id.clone(), name.clone());
        }
    }

    // Map back out and assert the content array is semantically identical.
    let rebuilt_blocks: Vec<Value> = parts
        .iter()
        .map(|p| part_to_block(p, &server_tool_names))
        .collect();
    assert_eq!(
        &Value::Array(rebuilt_blocks),
        &fixture["content"],
        "content blocks did not round-trip losslessly"
    );

    // Usage: typed fields are extracted correctly, and the raw payload is
    // preserved so a replay is byte-equivalent.
    let usage = usage_from_native(&fixture["usage"]);
    assert_eq!(usage.input_tokens, Some(1024));
    assert_eq!(usage.output_tokens, Some(128));
    assert_eq!(usage.cache_read_tokens, Some(512));
    assert_eq!(usage.cache_write_tokens, Some(64));
    assert_eq!(usage.server_tool_use.get("web_search_requests"), Some(&1));
    assert_eq!(usage.raw.as_ref(), Some(&fixture["usage"]));

    // The whole domain-model message also survives its own serde round-trip.
    let message = loom_core::Message {
        role: loom_core::Role::Assistant,
        content: parts,
        usage: Some(usage),
    };
    let encoded = serde_json::to_string(&message).expect("serialize message");
    let decoded: loom_core::Message = serde_json::from_str(&encoded).expect("deserialize message");
    assert_eq!(message, decoded);
}
