//! Translation between Loom's fluent conversation model and Anthropic's native
//! Messages API wire format.
//!
//! The translation is **lossless and provider-faithful** in both directions:
//!
//! - [`translate_request`] maps a [`Conversation`] plus its request-time
//!   [`ConversationOptions`] to the native `POST /v1/messages` request body,
//!   mapping each [`ContentPart`] to its native content block (the inverse of
//!   [`translate_response`]), carrying `thinking` / `redacted_thinking` blocks
//!   through unchanged for multi-turn correctness, and merging the
//!   `provider_options["anthropic"]` bag over the request so callers can pass
//!   any native field (`tool_choice`, `top_p`, thinking config, beta flags, …)
//!   without a Loom release.
//! - [`translate_response`] maps a native Messages response back to an
//!   assistant [`Message`], mapping content blocks to [`ContentPart`]s — with
//!   **unknown block types preserved verbatim** via
//!   [`ContentPart::ProviderExtension`], never an error — and populating
//!   [`Message::raw`] with the verbatim native response for audit and replay.
//!
//! These functions are pure and free of I/O, so they can be exercised directly
//! against recorded fixtures.

use std::collections::BTreeMap;

use loom_core::{
    Citation, ContentPart, Conversation, ConversationOptions, MediaSource, Message, Role,
    ToolDefinition, Usage,
};
use loom_provider::StopReason;
use serde_json::{json, Map, Value};

/// The `max_tokens` used when the caller does not specify one.
///
/// Anthropic requires `max_tokens` on every request; this is a conservative
/// default for callers who leave [`ConversationOptions::max_tokens`] unset.
const DEFAULT_MAX_TOKENS: u64 = 4096;

/// Maps a [`Conversation`] and its [`ConversationOptions`] to a native
/// Anthropic Messages API request body.
#[must_use]
pub fn translate_request(conversation: &Conversation, options: &ConversationOptions) -> Value {
    let mut root = Map::new();
    root.insert("model".into(), json!(conversation.binding.model));
    root.insert(
        "max_tokens".into(),
        json!(options.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
    );

    if let Some(system) = &conversation.system {
        root.insert("system".into(), json!(system));
    }

    // Correlate server-tool results with their originating calls so a
    // ServerToolResult can reconstruct its native `<name>_tool_result` block.
    let server_tool_names = server_tool_name_map(conversation);

    let messages = conversation
        .messages
        .iter()
        .map(|message| message_to_native(message, &server_tool_names))
        .collect();
    root.insert("messages".into(), Value::Array(messages));

    if !options.tools.is_empty() {
        let tools = options.tools.iter().map(tool_to_native).collect();
        root.insert("tools".into(), Value::Array(tools));
    }

    if let Some(temperature) = options.temperature {
        root.insert("temperature".into(), json!(temperature));
    }

    if !options.stop_sequences.is_empty() {
        root.insert("stop_sequences".into(), json!(options.stop_sequences));
    }

    // Merge the native options bag transparently, last-write-wins, so callers
    // can pass — or override — anything Anthropic accepts.
    if let Some(Value::Object(bag)) = options.provider_options.get("anthropic") {
        for (key, value) in bag {
            root.insert(key.clone(), value.clone());
        }
    }

    Value::Object(root)
}

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

/// Maps a native Anthropic `usage` object into [`Usage`].
///
/// Anthropic's `cache_creation_input_tokens` maps to
/// [`Usage::cache_write_tokens`] and `cache_read_input_tokens` to
/// [`Usage::cache_read_tokens`]. Per-server-tool counters are carried in
/// [`Usage::server_tool_use`], and the whole native object is preserved in
/// [`Usage::raw`] so no provider-specific figure is dropped.
#[must_use]
pub fn translate_usage(native: &Value) -> Usage {
    let mut usage = Usage::new();
    usage.input_tokens = native.get("input_tokens").and_then(Value::as_u64);
    usage.output_tokens = native.get("output_tokens").and_then(Value::as_u64);
    usage.cache_read_tokens = native
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64);
    usage.cache_write_tokens = native
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64);
    if let Some(counters) = native.get("server_tool_use").and_then(Value::as_object) {
        usage.server_tool_use = counters
            .iter()
            .filter_map(|(name, value)| value.as_u64().map(|count| (name.clone(), count)))
            .collect();
    }
    usage.raw = Some(native.clone());
    usage
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

/// Builds a map from a server-tool `tool_use_id` to the tool's name across the
/// whole conversation.
fn server_tool_name_map(conversation: &Conversation) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for message in &conversation.messages {
        for part in &message.content {
            if let ContentPart::ServerToolUse { id, name, .. } = part {
                map.insert(id.clone(), name.clone());
            }
        }
    }
    map
}

/// Maps a Loom [`Message`] into a native Anthropic message object.
fn message_to_native(message: &Message, server_tool_names: &BTreeMap<String, String>) -> Value {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        // Provider-authored content (server tool use/results injected by the
        // provider) belongs to the model's turn in Anthropic's wire format.
        Role::Provider => "assistant",
        _ => "user",
    };
    let content = message
        .content
        .iter()
        .map(|part| part_to_block(part, server_tool_names))
        .collect();
    json!({ "role": role, "content": Value::Array(content) })
}

/// Maps a [`ToolDefinition`] into a native Anthropic tool object.
fn tool_to_native(tool: &ToolDefinition) -> Value {
    let mut obj = Map::new();
    obj.insert("name".into(), json!(tool.name));
    if let Some(description) = &tool.description {
        obj.insert("description".into(), json!(description));
    }
    obj.insert("input_schema".into(), tool.input_schema.clone());
    Value::Object(obj)
}

/// Serialises a [`MediaSource`] to its native Anthropic `source` object.
///
/// The domain type's serde representation already matches Anthropic's shape
/// (`{ "type": "base64", … }` / `{ "type": "url", … }`), so it is emitted
/// directly.
fn media_source(source: &MediaSource) -> Value {
    serde_json::to_value(source).unwrap_or(Value::Null)
}

/// Maps a [`ContentPart`] out to its native Anthropic content block.
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
        ContentPart::Image { source } => {
            json!({ "type": "image", "source": media_source(source) })
        }
        ContentPart::Document { source } => {
            json!({ "type": "document", "source": media_source(source) })
        }
        ContentPart::ToolUse { id, name, input } => {
            json!({ "type": "tool_use", "id": id, "name": name, "input": input })
        }
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let mut obj = Map::new();
            obj.insert("type".into(), json!("tool_result"));
            obj.insert("tool_use_id".into(), json!(tool_use_id));
            obj.insert("content".into(), content.clone());
            if let Some(is_error) = is_error {
                obj.insert("is_error".into(), json!(is_error));
            }
            Value::Object(obj)
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
        // The escape hatch carries the native block verbatim.
        ContentPart::ProviderExtension { payload, .. } => payload.clone(),
        // `ContentPart` is `#[non_exhaustive]`; a future variant Loom does not
        // yet map is emitted as an empty object rather than panicking.
        _ => json!({}),
    }
}

/// Maps a native Anthropic content block into a [`ContentPart`].
///
/// Unknown block types fall through to [`ContentPart::ProviderExtension`] so no
/// provider feature is ever dropped.
fn block_to_part(block: &Value) -> ContentPart {
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
        },
        "thinking" => ContentPart::Thinking {
            thinking: str_field(block, "thinking"),
            signature: block
                .get("signature")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        "redacted_thinking" => ContentPart::RedactedThinking {
            data: str_field(block, "data"),
        },
        "tool_use" => ContentPart::ToolUse {
            id: str_field(block, "id"),
            name: str_field(block, "name"),
            input: block.get("input").cloned().unwrap_or(Value::Null),
        },
        // A client tool_result (sent by the host on a following user turn).
        // Matched before the server `<tool>_tool_result` arm.
        "tool_result" => ContentPart::ToolResult {
            tool_use_id: str_field(block, "tool_use_id"),
            content: block.get("content").cloned().unwrap_or(Value::Null),
            is_error: block.get("is_error").and_then(Value::as_bool),
        },
        "server_tool_use" => ContentPart::ServerToolUse {
            id: str_field(block, "id"),
            name: str_field(block, "name"),
            input: block.get("input").cloned().unwrap_or(Value::Null),
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
