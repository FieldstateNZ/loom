//! Mapping a native Anthropic `usage` object to Loom's [`Usage`].

use loom_core::Usage;
use serde_json::Value;

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
