//! Mapping a native Anthropic Messages response back to Loom's assistant
//! [`Message`], and the shared native-block → [`ContentPart`] mapping reused by
//! the streaming accumulator.

use loom_core::{Citation, ContentPart, Message, Role};
use loom_provider::StopReason;
use serde_json::Value;

use super::cache_control::cache_hint_from_native;
use super::usage::translate_usage;

/// Maps a native Anthropic Messages response into an assistant [`Message`],
/// preserving the verbatim response in [`Message::raw`].
#[must_use]
pub fn translate_response(native: &Value) -> Message {
    let content = native
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| blocks.iter().map(block_to_part).collect())
        .unwrap_or_default();

    let usage = native.get("usage").map(translate_usage);

    let role = match native.get("role").and_then(Value::as_str) {
        Some("user") => Role::User,
        _ => Role::Assistant,
    };

    Message {
        role,
        content,
        usage,
        raw: Some(native.clone()),
    }
}

/// Maps a native Anthropic `stop_reason` string to a [`StopReason`].
///
/// Returns `None` when the response carries no `stop_reason`. Any value Loom
/// does not model as a typed variant is preserved verbatim in
/// [`StopReason::Other`].
#[must_use]
pub fn stop_reason(native: &Value) -> Option<StopReason> {
    native
        .get("stop_reason")
        .and_then(Value::as_str)
        .map(map_stop_reason)
}

fn map_stop_reason(reason: &str) -> StopReason {
    match reason {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        "tool_use" => StopReason::ToolUse,
        "pause_turn" => StopReason::PauseTurn,
        "refusal" => StopReason::Refusal,
        other => StopReason::Other(other.to_owned()),
    }
}

/// Maps a native Anthropic content block into a [`ContentPart`].
///
/// Unknown block types fall through to [`ContentPart::ProviderExtension`] so no
/// provider feature is ever dropped. This is the single, shared block→part
/// mapping used by both the non-streaming [`translate_response`] path and the
/// streaming accumulator, so a block assembled from SSE deltas maps identically
/// to the same block delivered whole. A native `cache_control` marker (which
/// Anthropic may echo on a block) is read back onto the domain [`CacheHint`].
///
/// [`CacheHint`]: loom_core::CacheHint
#[must_use]
pub fn block_to_part(block: &Value) -> ContentPart {
    match block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "text" => ContentPart::Text {
            text: str_field(block, "text"),
            citations: block
                .get("citations")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().map(|value| Citation(value.clone())).collect()),
            cache: cache_hint_from_native(block),
        },
        "thinking" => ContentPart::Thinking {
            thinking: str_field(block, "thinking"),
            signature: block
                .get("signature")
                .and_then(Value::as_str)
                .map(str::to_owned),
            cache: cache_hint_from_native(block),
        },
        "redacted_thinking" => ContentPart::RedactedThinking {
            data: str_field(block, "data"),
        },
        "tool_use" => ContentPart::ToolUse {
            id: str_field(block, "id"),
            name: str_field(block, "name"),
            input: block.get("input").cloned().unwrap_or(Value::Null),
            cache: cache_hint_from_native(block),
        },
        // A client tool_result (sent by the host on a following user turn).
        // Matched before the server `<tool>_tool_result` arm.
        "tool_result" => ContentPart::ToolResult {
            tool_use_id: str_field(block, "tool_use_id"),
            content: block.get("content").cloned().unwrap_or(Value::Null),
            is_error: block.get("is_error").and_then(Value::as_bool),
            cache: cache_hint_from_native(block),
        },
        "server_tool_use" => ContentPart::ServerToolUse {
            id: str_field(block, "id"),
            name: str_field(block, "name"),
            input: block.get("input").cloned().unwrap_or(Value::Null),
        },
        // MCP connector blocks carry provider-specific fields (`server_name`,
        // `is_error`, …) that Loom's typed server-tool parts do not model, so
        // they ride through the escape hatch **verbatim** rather than being
        // reshaped — this keeps MCP tool use/results lossless and round-trip
        // exact. `mcp_tool_result` is matched here, before the generic
        // `<tool>_tool_result` arm below, so its extra fields are not dropped.
        "mcp_tool_use" | "mcp_tool_result" => ContentPart::ProviderExtension {
            provider: crate::PROVIDER_NAME.to_owned(),
            kind: block
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            payload: block.clone(),
        },
        // Provider-executed tool results share a `<tool>_tool_result` naming
        // convention; the result payload is preserved verbatim.
        other if other.ends_with("_tool_result") => ContentPart::ServerToolResult {
            tool_use_id: str_field(block, "tool_use_id"),
            content: block.get("content").cloned().unwrap_or(Value::Null),
        },
        // Anything not modelled natively rides through the escape hatch whole.
        other => ContentPart::ProviderExtension {
            provider: crate::PROVIDER_NAME.to_owned(),
            kind: other.to_owned(),
            payload: block.clone(),
        },
    }
}

/// Reads a string field from a native block, defaulting to empty when absent.
fn str_field(block: &Value, key: &str) -> String {
    block
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
