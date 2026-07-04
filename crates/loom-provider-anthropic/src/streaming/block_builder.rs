//! [`BlockBuilder`]: the accumulator for a single content block's streamed
//! deltas.

use serde_json::{json, Value};

/// Accumulator for a single content block's streamed deltas.
#[derive(Debug, Default)]
pub(super) struct BlockBuilder {
    /// The initial block from `content_block_start` (carries `type`, and any
    /// metadata such as a tool-use `id`/`name` or existing `citations`).
    pub(super) initial: Value,
    /// Concatenated `text_delta` fragments.
    pub(super) text: String,
    /// Concatenated `input_json_delta` fragments (parsed at finalisation).
    pub(super) partial_json: String,
    /// Concatenated `thinking_delta` fragments.
    pub(super) thinking: String,
    /// Concatenated `signature_delta` fragments.
    pub(super) signature: String,
    /// Citations appended to a text block via `citations_delta`, preserved
    /// verbatim in arrival order.
    pub(super) citations: Vec<Value>,
}

impl BlockBuilder {
    pub(super) fn new(initial: Value) -> Self {
        Self {
            initial,
            ..Self::default()
        }
    }

    /// Produces the fully-assembled native block from the initial shape plus
    /// the accumulated deltas.
    pub(super) fn finalized(&self) -> Value {
        let mut block = self.initial.clone();
        let Value::Object(map) = &mut block else {
            return block;
        };
        match map.get("type").and_then(Value::as_str).unwrap_or_default() {
            "text" => {
                map.insert("text".into(), json!(self.text));
                if !self.citations.is_empty() {
                    map.insert("citations".into(), Value::Array(self.citations.clone()));
                }
            }
            "thinking" => {
                map.insert("thinking".into(), json!(self.thinking));
                if !self.signature.is_empty() {
                    map.insert("signature".into(), json!(self.signature));
                }
            }
            kind if (kind == "tool_use"
                || kind == "server_tool_use"
                || kind.ends_with("_tool_use"))
                && !self.partial_json.is_empty() =>
            {
                let input = serde_json::from_str::<Value>(&self.partial_json)
                    .unwrap_or_else(|_| json!(self.partial_json));
                map.insert("input".into(), input);
            }
            _ => {}
        }
        block
    }
}
