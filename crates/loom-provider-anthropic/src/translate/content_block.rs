//! Mapping a Loom [`ContentPart`] out to its native Anthropic content block —
//! the request-direction counterpart of
//! [`block_to_part`](super::response::block_to_part).

use std::collections::BTreeMap;

use loom_core::{ContentPart, MediaSource};
use serde_json::{json, Map, Value};

use super::cache_control::attach_cache;

/// Maps a [`ContentPart`] out to its native Anthropic content block, attaching
/// a `cache_control` marker for the cacheable variants that carry a hint.
pub(super) fn part_to_block(
    part: &ContentPart,
    server_tool_names: &BTreeMap<String, String>,
) -> Value {
    match part {
        ContentPart::Text {
            text,
            citations,
            cache,
        } => {
            let mut obj = Map::new();
            obj.insert("type".into(), json!("text"));
            obj.insert("text".into(), json!(text));
            if let Some(citations) = citations {
                obj.insert(
                    "citations".into(),
                    Value::Array(citations.iter().map(|c| c.0.clone()).collect()),
                );
            }
            attach_cache(Value::Object(obj), cache.as_ref())
        }
        ContentPart::Image { source, cache } => attach_cache(
            json!({ "type": "image", "source": media_source(source) }),
            cache.as_ref(),
        ),
        ContentPart::Document { source, cache } => attach_cache(
            json!({ "type": "document", "source": media_source(source) }),
            cache.as_ref(),
        ),
        ContentPart::ToolUse {
            id,
            name,
            input,
            cache,
        } => attach_cache(
            json!({ "type": "tool_use", "id": id, "name": name, "input": input }),
            cache.as_ref(),
        ),
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error,
            cache,
        } => {
            let mut obj = Map::new();
            obj.insert("type".into(), json!("tool_result"));
            obj.insert("tool_use_id".into(), json!(tool_use_id));
            obj.insert("content".into(), content.clone());
            if let Some(is_error) = is_error {
                obj.insert("is_error".into(), json!(is_error));
            }
            attach_cache(Value::Object(obj), cache.as_ref())
        }
        ContentPart::ServerToolUse { id, name, input } => {
            json!({ "type": "server_tool_use", "id": id, "name": name, "input": input })
        }
        ContentPart::ServerToolResult {
            tool_use_id,
            content,
        } => {
            // Reconstruct the native `<name>_tool_result` block type from the
            // originating server_tool_use call; fall back to a generic
            // `tool_result` if the correlation is missing.
            let block_type = server_tool_names.get(tool_use_id).map_or_else(
                || "tool_result".to_owned(),
                |name| format!("{name}_tool_result"),
            );
            json!({ "type": block_type, "tool_use_id": tool_use_id, "content": content })
        }
        ContentPart::Thinking {
            thinking,
            signature,
            cache,
        } => {
            let mut obj = Map::new();
            obj.insert("type".into(), json!("thinking"));
            obj.insert("thinking".into(), json!(thinking));
            if let Some(signature) = signature {
                obj.insert("signature".into(), json!(signature));
            }
            attach_cache(Value::Object(obj), cache.as_ref())
        }
        ContentPart::RedactedThinking { data } => {
            json!({ "type": "redacted_thinking", "data": data })
        }
        // The escape hatch carries the native block verbatim.
        ContentPart::ProviderExtension { payload, .. } => payload.clone(),
        // `ContentPart` is `#[non_exhaustive]`; a future variant Loom does not
        // yet map is emitted as an empty object rather than panicking.
        _ => json!({}),
    }
}

/// Serialises a [`MediaSource`] to its native Anthropic `source` object.
///
/// The domain type's serde representation already matches Anthropic's shape
/// (`{ "type": "base64", … }` / `{ "type": "url", … }`), so it is emitted
/// directly.
fn media_source(source: &MediaSource) -> Value {
    serde_json::to_value(source).unwrap_or(Value::Null)
}
