//! Mapping a [`Conversation`] and its [`ConversationOptions`] to a native
//! Anthropic Messages API request body.

use std::collections::BTreeMap;

use loom_core::{
    CacheHint, ContentPart, Conversation, ConversationOptions, Message, Role, ToolDefinition,
};
use serde_json::{json, Map, Value};

use super::betas::RESERVED_BETAS_KEY;
use super::cache_control::{apply_auto_cache, cache_hint_to_native};
use super::content_block::part_to_block;
use super::server_tools::{mcp_server_to_native, server_tool_to_native};

/// The `max_tokens` used when the caller does not specify one.
///
/// Anthropic requires `max_tokens` on every request; this is a conservative
/// default for callers who leave [`ConversationOptions::max_tokens`] unset.
const DEFAULT_MAX_TOKENS: u64 = 4096;

/// Maps a [`Conversation`] and its [`ConversationOptions`] to a native
/// Anthropic Messages API request body.
///
/// Explicit [`CacheHint`]s on content parts, tool definitions, and the system
/// prompt are emitted as native `cache_control` markers. When
/// [`ConversationOptions::auto_cache`] is set, up to two further deterministic
/// breakpoints are placed (see `apply_auto_cache`, private, in the
/// `cache_control` submodule).
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

/// Emits the native `system` field.
///
/// Anthropic accepts `system` as a plain string or an array of text blocks. A
/// plain string is emitted unless a cache breakpoint applies: an explicit
/// `cache` hint produces a single text block carrying `cache_control`, and
/// `force_block` (set when auto-cache is enabled) produces a bare text block
/// that the auto-cache pass can later mark.
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
