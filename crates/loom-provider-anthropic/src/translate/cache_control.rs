//! Prompt-cache `cache_control` markers: mapping [`CacheHint`] to and from
//! Anthropic's native representation, stripping markers for cache negotiation,
//! and placing the deterministic auto-cache breakpoints.

use loom_core::{CacheHint, CacheTtl, ContentPart, Conversation, ConversationOptions};
use serde_json::{json, Map, Value};

/// Anthropic's maximum number of `cache_control` breakpoints per request.
const MAX_CACHE_BREAKPOINTS: usize = 4;

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

/// Maps a [`CacheHint`] to Anthropic's native `cache_control` object.
///
/// A hint with no explicit lifetime maps to the default
/// `{ "type": "ephemeral" }`; a [`CacheTtl`] maps to Anthropic's `"5m"` / `"1h"`
/// `ttl` values.
pub(super) fn cache_hint_to_native(hint: &CacheHint) -> Value {
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
pub(super) fn cache_hint_from_native(block: &Value) -> Option<CacheHint> {
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
pub(super) fn attach_cache(mut block: Value, cache: Option<&CacheHint>) -> Value {
    if let (Value::Object(map), Some(hint)) = (&mut block, cache) {
        map.insert("cache_control".into(), cache_hint_to_native(hint));
    }
    block
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
pub(super) fn apply_auto_cache(root: &mut Map<String, Value>, conversation: &Conversation) {
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
