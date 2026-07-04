//! [`BatchSubmitItem`]: one item to submit to a provider batch.

use loom_core::{Conversation, ConversationOptions};

/// One item to submit to a provider batch: a correlation id plus the
/// (provider-agnostic) conversation to run. Translation to the provider's native
/// request shape happens inside the [`BatchBackend`](super::BatchBackend).
#[derive(Clone, Debug)]
pub struct BatchSubmitItem {
    /// The per-item correlation id, echoed on the matching result.
    pub custom_id: String,
    /// The conversation to run.
    pub conversation: Conversation,
    /// The request-time options.
    pub options: ConversationOptions,
}
