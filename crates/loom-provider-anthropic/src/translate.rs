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

use std::collections::{BTreeMap, BTreeSet};

use loom_core::{
    CacheHint, CacheTtl, Citation, ContentPart, Conversation, ConversationOptions, McpServerRef,
    MediaSource, Message, Role, ServerTool, ToolDefinition, Usage,
};
use loom_provider::StopReason;
use serde_json::{json, Map, Value};

use crate::catalogue::{feature_beta, BetaFeature};

/// The native `type` for Anthropic's web search server tool.
const WEB_SEARCH_TOOL_TYPE: &str = "web_search_20250305";
/// The native `type` for Anthropic's code execution server tool.
const CODE_EXECUTION_TOOL_TYPE: &str = "code_execution_20250522";
/// The reserved `provider_options["anthropic"]` key carrying caller-supplied
/// `anthropic-beta` tokens. It is consumed for the request header and never
/// merged into the request body.
const RESERVED_BETAS_KEY: &str = "betas";

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

    // Client tools and server (provider-executed) tools share the native
    // `tools` array; server-tool entries are native versioned tool definitions.
    let mut tools: Vec<Value> = options.tools.iter().map(tool_to_native).collect();
    tools.extend(options.server_tools.iter().map(server_tool_to_native));
    if !tools.is_empty() {
        root.insert("tools".into(), Value::Array(tools));
    }

    // External MCP servers the model may call through Anthropic's connector.
    // For a named reference the URL and authorization token are resolved and
    // injected upstream (in loom-server), so by the time a ref reaches here it
    // carries whatever the request should send verbatim.
    if !options.mcp_servers.is_empty() {
        let servers: Vec<Value> = options
            .mcp_servers
            .iter()
            .map(mcp_server_to_native)
            .collect();
        root.insert("mcp_servers".into(), Value::Array(servers));
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
    // can pass — or override — anything Anthropic accepts. The reserved `betas`
    // key is a header directive, not a body field, so it is skipped here (see
    // `required_betas`).
    if let Some(Value::Object(bag)) = options.provider_options.get("anthropic") {
        for (key, value) in bag {
            if key == RESERVED_BETAS_KEY {
                continue;
            }
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

/// Computes the set of `anthropic-beta` tokens a request requires, deterministic
/// and de-duplicated.
///
/// The set is the union of:
///
/// - the catalogue-driven default token for each server-tool feature the
///   request uses (see [`feature_beta`]); and
/// - any tokens the caller supplied verbatim through the reserved
///   `provider_options["anthropic"]["betas"]` array.
///
/// This is the mechanism that lets a new beta be adopted **without a Loom
/// release**: a caller adds the token to `betas` (to add), and can override a
/// stale default by disabling auto-derivation on the provider and supplying the
/// full set. The [`AnthropicProvider`](crate::AnthropicProvider) merges these
/// with any betas configured on the provider itself and sends them as the
/// `anthropic-beta` header.
#[must_use]
pub fn required_betas(_conversation: &Conversation, options: &ConversationOptions) -> Vec<String> {
    let mut betas = BTreeSet::new();

    for tool in &options.server_tools {
        let feature = match tool {
            ServerTool::WebSearch { .. } => Some(BetaFeature::WebSearch),
            ServerTool::CodeExecution { .. } => Some(BetaFeature::CodeExecution),
            // A `Raw` passthrough's beta (if any) is the caller's responsibility
            // via the `betas` override; Loom cannot infer it.
            _ => None,
        };
        if let Some(token) = feature.and_then(feature_beta) {
            betas.insert(token.to_owned());
        }
    }

    // Attaching external MCP servers requires the connector's beta flag.
    if !options.mcp_servers.is_empty() {
        if let Some(token) = feature_beta(BetaFeature::McpConnector) {
            betas.insert(token.to_owned());
        }
    }

    for token in configured_betas(options) {
        betas.insert(token);
    }

    betas.into_iter().collect()
}

/// Reads caller-supplied `anthropic-beta` tokens from the reserved
/// `provider_options["anthropic"]["betas"]` array, ignoring non-string entries.
fn configured_betas(options: &ConversationOptions) -> Vec<String> {
    options
        .provider_options
        .get("anthropic")
        .and_then(|bag| bag.get(RESERVED_BETAS_KEY))
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
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

/// Maps a [`ServerTool`] to its native Anthropic versioned tool entry.
///
/// [`ServerTool::WebSearch`] and [`ServerTool::CodeExecution`] map to their
/// native `{ "type": "<versioned>", "name": … }` shapes; [`ServerTool::Raw`]
/// forwards its wrapped native definition verbatim so a caller can drive a
/// server tool Loom does not model yet.
fn server_tool_to_native(tool: &ServerTool) -> Value {
    match tool {
        ServerTool::WebSearch {
            max_uses,
            allowed_domains,
            blocked_domains,
        } => {
            let mut obj = Map::new();
            obj.insert("type".into(), json!(WEB_SEARCH_TOOL_TYPE));
            obj.insert("name".into(), json!("web_search"));
            if let Some(max_uses) = max_uses {
                obj.insert("max_uses".into(), json!(max_uses));
            }
            if let Some(allowed_domains) = allowed_domains {
                obj.insert("allowed_domains".into(), json!(allowed_domains));
            }
            if let Some(blocked_domains) = blocked_domains {
                obj.insert("blocked_domains".into(), json!(blocked_domains));
            }
            Value::Object(obj)
        }
        ServerTool::CodeExecution {} => {
            json!({ "type": CODE_EXECUTION_TOOL_TYPE, "name": "code_execution" })
        }
        // The escape hatch carries the native tool definition verbatim.
        ServerTool::Raw(definition) => definition.clone(),
        // `ServerTool` is `#[non_exhaustive]`; a future variant Loom does not
        // yet map is emitted as an empty object rather than panicking.
        _ => json!({}),
    }
}

/// Maps an [`McpServerRef`] to Anthropic's native `mcp_servers` entry.
///
/// Anthropic's connector expects `{ "type": "url", "name", "url",
/// "authorization_token"?, "tool_configuration"? }`. The authorization token is
/// emitted only when present — for a named reference it has been injected
/// upstream after decryption; for an inline reference the caller supplied it.
/// The token is a bearer secret and appears **only** in the outbound request
/// body, never in a response or in persisted history.
fn mcp_server_to_native(server: &McpServerRef) -> Value {
    let mut obj = Map::new();
    obj.insert("type".into(), json!("url"));
    obj.insert("name".into(), json!(server.name));
    if let Some(url) = &server.url {
        obj.insert("url".into(), json!(url));
    }
    if let Some(authorization) = &server.authorization {
        obj.insert("authorization_token".into(), json!(authorization));
    }
    if let Some(tool_configuration) = &server.tool_configuration {
        obj.insert("tool_configuration".into(), tool_configuration.clone());
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
