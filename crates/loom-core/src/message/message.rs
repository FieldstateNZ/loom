//! A single turn in a conversation.

use serde::{Deserialize, Serialize};

use super::Role;
use crate::{ContentPart, Usage};

/// A single turn in a conversation.
///
/// A message carries an ordered list of [`ContentPart`]s (order is
/// significant — providers interleave text, tool calls, and reasoning) and,
/// for assistant/provider turns, an optional [`Usage`] snapshot describing what
/// the response cost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// The author of the message.
    pub role: Role,

    /// The message's content, in provider-significant order.
    #[serde(default)]
    pub content: Vec<ContentPart>,

    /// Resource usage reported for this message, when the provider supplied it
    /// (typically only on assistant/provider responses).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,

    /// The provider's verbatim native response payload, preserved for audit and
    /// byte-equivalent replay.
    ///
    /// Populated by provider translators on the assistant/provider turns they
    /// synthesise from a native response, so the exact bytes the provider
    /// returned are never lost even where Loom models the response as typed
    /// [`ContentPart`]s. `None` for messages Loom (or a host application)
    /// constructs itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

impl Message {
    /// Constructs a message with the given role and content and no usage
    /// snapshot.
    #[must_use]
    pub fn new(role: Role, content: Vec<ContentPart>) -> Self {
        Self {
            role,
            content,
            usage: None,
            raw: None,
        }
    }

    /// Constructs a [`Role::User`] message from a single string of text.
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, vec![ContentPart::text(text)])
    }

    /// Constructs a [`Role::Assistant`] message from a single string of text.
    #[must_use]
    pub fn assistant(text: impl Into<String>) -> Self {
        Self::new(Role::Assistant, vec![ContentPart::text(text)])
    }
}
