//! Messages and the roles that author them.

use serde::{Deserialize, Serialize};

use crate::{ContentPart, Usage};

/// Who authored a [`Message`].
///
/// `#[non_exhaustive]` because providers may introduce further roles (e.g. an
/// operator or system channel distinct from the ones modelled here).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Role {
    /// A message authored by the end user.
    User,
    /// A message authored by the model.
    Assistant,
    /// A message originating from the provider itself — for example a
    /// server-executed tool result injected into the conversation.
    Provider,
}

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
