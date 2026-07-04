//! Who authored a message.

use serde::{Deserialize, Serialize};

/// Who authored a [`Message`](super::Message).
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
