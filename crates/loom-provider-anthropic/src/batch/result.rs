//! [`AnthropicBatchResult`]: one line of a batch's JSONL results document.

use serde_json::Value;

/// One line of a batch's JSONL results document: a request's `custom_id` and its
/// verbatim `result` object (`{ "type": "succeeded", "message": … }`,
/// `{ "type": "errored", "error": … }`, `canceled`, or `expired`).
#[derive(Clone, Debug, PartialEq)]
pub struct AnthropicBatchResult {
    /// The correlation id of the request this result belongs to.
    pub custom_id: String,
    /// The verbatim result object.
    pub result: Value,
}
