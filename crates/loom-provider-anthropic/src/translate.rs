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
//! # Prompt caching
//!
//! A provider-agnostic [`CacheHint`] on a cacheable [`ContentPart`], a
//! [`ToolDefinition`], or the conversation's system prompt maps to Anthropic's
//! native `cache_control: { "type": "ephemeral"[, "ttl": "1h"] }` marker on the
//! corresponding native block, and is read back off the block on response
//! translation. When [`ConversationOptions::auto_cache`] is set, Loom
//! additionally places up to two deterministic breakpoints — after the stable
//! system-plus-tools head and on the trailing history boundary — respecting
//! Anthropic's maximum of four cache breakpoints per request. See
//! [`apply_auto_cache`] (private) and [`strip_cache_control`].
//!
//! These functions are pure and free of I/O, so they can be exercised directly
//! against recorded fixtures.

use std::collections::BTreeMap;

use loom_core::{
    CacheHint, CacheTtl, Citation, ContentPart, Conversation, ConversationOptions, MediaSource,
    Message, Role, ToolDefinition, Usage,
};
use loom_provider::StopReason;
use serde_json::{json, Map, Value};

/// The `max_tokens` used when the caller does not specify one.
///
/// Anthropic requires `max_tokens` on every request; this is a conservative
/// default for callers who leave [`ConversationOptions::max_tokens`] unset.
const DEFAULT_MAX_TOKENS: u64 = 4096;

/// Anthropic's maximum number of `cache_control` breakpoints per request.
const MAX_CACHE_BREAKPOINTS: usize = 4;

/// Maps a [`Conversation`] and its [`ConversationOptions`] to a native
/// Anthropic Messages API request body.
///
/// Explicit [`CacheHint`]s on content parts, tool definitions, and the system
/// prompt are emitted as native `cache_control` markers. When
/// [`ConversationOptions::auto_cache`] is set, up to two further deterministic
/// breakpoints are placed (see [`apply_auto_cache`]).
#[must_use]
pub fn translate_request(conversation: &Conversation, options: &ConversationOptions) -> Value {
    let mut root = Map::new();
    root.insert("model".into(), json!(conversation.binding.model));
    root.insert(
        "max_tokens".into(),
        json!(options.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
    );

    if let Some(system) = &conversation.system {
        // Emit the system prompt in block form when a cache breakpoint may land
        // on it — either explicitly (`system_cache`) or via auto-cache — so the
        // marker has a block to attach to; otherwise a plain string suffices.
        let as_block = conversation.system_cache.is_some() || options.auto_cache;
        root.insert(
            "system".into(),
            system_to_native(system, conversation.system_cache.as_ref(), as_block),
        );
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

    if options.auto_cache {
        apply_auto_cache(&mut root, conversation);
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

/// Returns `true` if the request would carry any prompt-cache directive —
/// an explicit [`CacheHint`] on the system prompt, a tool, or a content part,
/// or the [`ConversationOptions::auto_cache`] flag.
///
/// The provider uses this to decide whether cache negotiation applies before
/// dispatching to a model that may not support prompt caching.
#[must_use]
pub fn requests_caching(conversation: &Conversation, options: &ConversationOptions) -> bool {
    if options.auto_cache || conversation.system_cache.is_some() {
        return true;
    }
    if options.tools.iter().any(|tool| tool.cache.is_some()) {
        return true;
    }
    conversation
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .any(|part| part_cache(part).is_some())
}

/// Recursively removes every `cache_control` marker from a native request body.
///
/// Used by the provider's soft-ignore cache negotiation path to drop advisory
/// cache hints for a model that does not support prompt caching, without
/// disturbing any other field.
pub fn strip_cache_control(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("cache_control");
            for child in map.values_mut() {
                strip_cache_control(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_cache_control(item);
            }
        }
        _ => {}
    }
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

/// Maps a [`CacheHint`] to Anthropic's native `cache_control` object.
///
/// A hint with no explicit lifetime maps to the default
/// `{ "type": "ephemeral" }`; a [`CacheTtl`] maps to Anthropic's `"5m"` / `"1h"`
/// `ttl` values.
fn cache_hint_to_native(hint: &CacheHint) -> Value {
    let mut obj = Map::new();
    obj.insert("type".into(), json!("ephemeral"));
    if let Some(ttl) = hint.ttl {
        obj.insert("ttl".into(), json!(cache_ttl_to_native(ttl)));
    }
    Value::Object(obj)
}

/// Maps a [`CacheTtl`] to Anthropic's `ttl` string.
fn cache_ttl_to_native(ttl: CacheTtl) -> &'static str {
    match ttl {
        CacheTtl::FiveMinutes => "5m",
        CacheTtl::OneHour => "1h",
        // `CacheTtl` is `#[non_exhaustive]`; an unrecognised future variant maps
        // conservatively to Anthropic's default short tier.
        _ => "5m",
    }
}

/// Reads a [`CacheHint`] back off a native block's `cache_control` object, if
/// present. Anthropic's `"5m"` / `"1h"` `ttl` values map to the typed
/// [`CacheTtl`]; a missing or unrecognised `ttl` yields the default hint.
fn cache_hint_from_native(block: &Value) -> Option<CacheHint> {
    let control = block.get("cache_control")?;
    if control.get("type").and_then(Value::as_str) != Some("ephemeral") {
        return None;
    }
    let hint = match control.get("ttl").and_then(Value::as_str) {
        Some("5m") => CacheHint::with_ttl(CacheTtl::FiveMinutes),
        Some("1h") => CacheHint::with_ttl(CacheTtl::OneHour),
        _ => CacheHint::ephemeral(),
    };
    Some(hint)
}

/// Emits the native `system` field.
///
/// Anthropic accepts `system` as a plain string or an array of text blocks. A
/// plain string is emitted unless a cache breakpoint applies: an explicit
/// `cache` hint produces a single text block carrying `cache_control`, and
/// `force_block` (set when auto-cache is enabled) produces a bare text block
/// that [`apply_auto_cache`] can later mark.
fn system_to_native(system: &str, cache: Option<&CacheHint>, force_block: bool) -> Value {
    if cache.is_none() && !force_block {
        return json!(system);
    }
    let mut block = Map::new();
    block.insert("type".into(), json!("text"));
    block.insert("text".into(), json!(system));
    if let Some(hint) = cache {
        block.insert("cache_control".into(), cache_hint_to_native(hint));
    }
    Value::Array(vec![Value::Object(block)])
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
    if let Some(hint) = &tool.cache {
        obj.insert("cache_control".into(), cache_hint_to_native(hint));
    }
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

/// Returns the [`CacheHint`] carried by a content part, if it is a cacheable
/// variant that has one set.
fn part_cache(part: &ContentPart) -> Option<&CacheHint> {
    match part {
        ContentPart::Text { cache, .. }
        | ContentPart::Image { cache, .. }
        | ContentPart::Document { cache, .. }
        | ContentPart::ToolUse { cache, .. }
        | ContentPart::ToolResult { cache, .. }
        | ContentPart::Thinking { cache, .. } => cache.as_ref(),
        _ => None,
    }
}

/// Attaches a native `cache_control` marker to a native block object when the
/// part carries a [`CacheHint`]. A no-op for non-object blocks or absent hints.
fn attach_cache(mut block: Value, cache: Option<&CacheHint>) -> Value {
    if let (Value::Object(map), Some(hint)) = (&mut block, cache) {
        map.insert("cache_control".into(), cache_hint_to_native(hint));
    }
    block
}

/// Maps a [`ContentPart`] out to its native Anthropic content block, attaching
/// a `cache_control` marker for the cacheable variants that carry a hint.
fn part_to_block(part: &ContentPart, server_tool_names: &BTreeMap<String, String>) -> Value {
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

/// Deterministically places up to two automatic cache breakpoints on an
/// already-built request body: one after the stable system-plus-tools head, and
/// one on the trailing history boundary.
///
/// The head breakpoint lands on the system block when a system prompt is
/// present (which caches any preceding tools with it, since tools render before
/// system), otherwise on the last tool definition. The trailing breakpoint
/// lands on the last content block of the last message. Auto breakpoints never
/// overwrite an explicit `cache_control`, and are only added while the request
/// stays within Anthropic's [`MAX_CACHE_BREAKPOINTS`] limit — explicit hints
/// are counted first so the caller's markers always take precedence.
fn apply_auto_cache(root: &mut Map<String, Value>, conversation: &Conversation) {
    let explicit = root.values().map(count_breakpoints).sum::<usize>();
    let mut budget = MAX_CACHE_BREAKPOINTS.saturating_sub(explicit);
    let default_control = cache_hint_to_native(&CacheHint::ephemeral());

    // Head breakpoint: prefer the system prompt (caches tools + system), else
    // the last tool definition.
    if budget > 0 {
        if let Some(Value::Array(system_blocks)) = root.get_mut("system") {
            if mark_last_uncached(system_blocks, &default_control) {
                budget -= 1;
            }
        } else if conversation.system.is_none() {
            if let Some(Value::Array(tools)) = root.get_mut("tools") {
                if mark_last_uncached(tools, &default_control) {
                    budget -= 1;
                }
            }
        }
    }

    // Trailing breakpoint: the last content block of the last message.
    if budget > 0 {
        if let Some(Value::Array(messages)) = root.get_mut("messages") {
            if let Some(Value::Object(last)) = messages.last_mut() {
                if let Some(Value::Array(content)) = last.get_mut("content") {
                    mark_last_uncached(content, &default_control);
                }
            }
        }
    }
}

/// Marks the last object in `blocks` with `control` unless it already carries a
/// `cache_control`. Returns `true` if a marker was added.
fn mark_last_uncached(blocks: &mut [Value], control: &Value) -> bool {
    if let Some(Value::Object(last)) = blocks.last_mut() {
        if !last.contains_key("cache_control") {
            last.insert("cache_control".into(), control.clone());
            return true;
        }
    }
    false
}

/// Counts the `cache_control` markers already present anywhere in a native
/// request body.
fn count_breakpoints(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            let here = usize::from(map.contains_key("cache_control"));
            here + map.values().map(count_breakpoints).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(count_breakpoints).sum(),
        _ => 0,
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
