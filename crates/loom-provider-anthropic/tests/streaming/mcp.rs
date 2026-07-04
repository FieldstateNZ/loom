//! MCP connector streaming: `mcp_tool_use` / `mcp_tool_result` blocks
//! reassembled via the provider-extension escape hatch.

use loom_core::ContentPart;
use loom_provider::TurnEventKind;
use loom_provider_anthropic::translate;
use serde_json::{json, Value};

use super::support::{drive, kinds};

#[test]
fn mcp_tool_use_and_result_stream_and_reassemble_via_provider_extension() {
    let raw = include_str!("../fixtures/stream_mcp.sse");
    let (results, accumulator) = drive(raw);
    assert!(results.iter().all(Result::is_ok));
    let kinds = kinds(&results);

    // The streamed mcp_tool_use assembles its input from partial-JSON deltas and
    // completes as a ProviderExtension that preserves the provider-specific
    // `server_name` field (which a typed server-tool part would drop).
    let complete0 = kinds
        .iter()
        .find_map(|k| match k {
            TurnEventKind::ContentPartComplete { index: 0, part } => Some(part.clone()),
            _ => None,
        })
        .expect("block 0 completes");
    match complete0 {
        ContentPart::ProviderExtension { kind, payload, .. } => {
            assert_eq!(kind, "mcp_tool_use");
            assert_eq!(payload["server_name"], json!("github"));
            assert_eq!(payload["input"], json!({ "query": "loom" }));
        }
        other => panic!("expected ProviderExtension, got {other:?}"),
    }

    // The mcp_tool_result arrives whole and rides through verbatim, keeping its
    // `is_error` flag.
    let complete1 = kinds
        .iter()
        .find_map(|k| match k {
            TurnEventKind::ContentPartComplete { index: 1, part } => Some(part.clone()),
            _ => None,
        })
        .expect("block 1 completes");
    match complete1 {
        ContentPart::ProviderExtension { kind, payload, .. } => {
            assert_eq!(kind, "mcp_tool_result");
            assert_eq!(payload["is_error"], json!(false));
        }
        other => panic!("expected ProviderExtension, got {other:?}"),
    }

    // The streamed-accumulated message is byte-for-byte the non-streaming one.
    let non_streaming: Value =
        serde_json::from_str(include_str!("../fixtures/stream_mcp_response.json"))
            .expect("valid fixture");
    assert_eq!(
        accumulator.message(),
        translate::translate_response(&non_streaming),
        "streamed MCP message must equal the non-streaming message"
    );
}
