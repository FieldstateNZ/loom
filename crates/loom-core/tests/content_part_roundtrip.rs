//! Serde round-trip tests for [`ContentPart`] and its self-describing tag.

mod common;

use common::assert_json_roundtrip;
use loom_core::{CacheHint, CacheTtl, Citation, ContentPart, MediaSource};
use serde_json::json;

#[test]
fn content_part_variants_roundtrip() {
    let parts = vec![
        ContentPart::Text {
            text: "plain".into(),
            citations: None,
            cache: None,
        },
        // A cache hint on a text block round-trips.
        ContentPart::Text {
            text: "cacheable prefix".into(),
            citations: None,
            cache: Some(CacheHint::ephemeral()),
        },
        ContentPart::Text {
            text: "cited".into(),
            citations: Some(vec![Citation(json!({
                "type": "char_location",
                "cited_text": "x",
                "start_char_index": 0,
                "end_char_index": 1
            }))]),
            cache: None,
        },
        ContentPart::Image {
            source: MediaSource::Base64 {
                media_type: "image/png".into(),
                data: "aGVsbG8=".into(),
            },
            cache: None,
        },
        ContentPart::Image {
            source: MediaSource::Url {
                url: "https://example.com/a.png".into(),
            },
            cache: Some(CacheHint::with_ttl(CacheTtl::OneHour)),
        },
        ContentPart::Document {
            source: MediaSource::Base64 {
                media_type: "application/pdf".into(),
                data: "JVBERi0=".into(),
            },
            cache: Some(CacheHint::with_ttl(CacheTtl::FiveMinutes)),
        },
        ContentPart::ToolUse {
            id: "toolu_1".into(),
            name: "get_weather".into(),
            input: json!({ "location": "Wellington" }),
            cache: None,
        },
        ContentPart::ToolResult {
            tool_use_id: "toolu_1".into(),
            content: json!("18C and windy"),
            is_error: None,
            cache: None,
        },
        ContentPart::ToolResult {
            tool_use_id: "toolu_1".into(),
            content: json!({ "error": "not found" }),
            is_error: Some(true),
            cache: Some(CacheHint::ephemeral()),
        },
        ContentPart::ServerToolUse {
            id: "srvtoolu_1".into(),
            name: "web_search".into(),
            input: json!({ "query": "loom gateway" }),
        },
        ContentPart::ServerToolResult {
            tool_use_id: "srvtoolu_1".into(),
            content: json!([{ "type": "web_search_result", "url": "https://x" }]),
        },
        ContentPart::Thinking {
            thinking: "let me reason".into(),
            signature: Some("sig-blob".into()),
            cache: None,
        },
        ContentPart::Thinking {
            thinking: String::new(),
            signature: None,
            cache: None,
        },
        ContentPart::RedactedThinking {
            data: "opaque==".into(),
        },
        ContentPart::ProviderExtension {
            provider: "anthropic".into(),
            kind: "mcp_tool_use".into(),
            payload: json!({ "id": "mcptoolu_1", "server_name": "github" }),
        },
    ];

    for part in &parts {
        assert_json_roundtrip(part);
    }
}

#[test]
fn text_part_type_tag_is_self_describing() {
    let value = serde_json::to_value(ContentPart::text("hi")).unwrap();
    assert_eq!(value["type"], json!("text"));
    assert_eq!(value["text"], json!("hi"));
    // Absent citations must not serialize (preserves provider omission).
    assert!(value.get("citations").is_none());
}
