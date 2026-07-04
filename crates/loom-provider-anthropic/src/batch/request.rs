//! [`BatchRequest`]: one request within a Message Batches submission.

use serde_json::Value;

/// One request within a batch: a caller-chosen correlation id plus the native
/// Messages request body to run.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchRequest {
    /// The caller-facing correlation id, echoed on the matching result. Unique
    /// within the batch.
    pub custom_id: String,
    /// The native Messages request body (as produced by
    /// [`translate_request`](crate::translate::translate_request)).
    pub params: Value,
}
