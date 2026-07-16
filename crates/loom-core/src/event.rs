//! The normalised, closed session-event vocabulary.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A session's run status, reported by a [`EventKind::Status`] event.
///
/// Distinct from [`SessionStatus`](crate::SessionStatus), which is the session's
/// durable lifecycle state (active vs superseded); this is its *execution* state
/// within the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunStatus {
    /// Actively producing output or executing a tool.
    Running,
    /// Settled, awaiting the next input.
    Idle,
    /// Hit an error (see the accompanying [`EventKind::Error`]).
    Error,
}

/// The payload of a normalised session [`Event`] — the closed vocabulary every
/// provider's streaming output is translated into.
///
/// Lossy by design: these eight variants are the whole vocabulary. A provider's
/// native event shapes are not preserved here (that fidelity lives on the
/// raw/passthrough path); what a conformant adapter MUST preserve is
/// pending-tool enumeration and event ordering.
///
/// Field names follow Loom's snake_case convention; the OASP wire form is
/// camelCase, mapped at the conformance boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventKind {
    /// The start of a new assistant message.
    AssistantMessageStart {
        /// Identifier of the assistant message beginning.
        message_id: String,
    },
    /// An incremental text chunk of an in-progress assistant message.
    AssistantMessageText {
        /// Identifier of the assistant message this chunk belongs to.
        message_id: String,
        /// The incremental text content of this chunk.
        delta: String,
    },
    /// The end of an assistant message.
    AssistantMessageEnd {
        /// Identifier of the assistant message ending.
        message_id: String,
    },
    /// An incremental chunk of the assistant's extended thinking.
    AssistantThinking {
        /// The incremental thinking content of this chunk.
        delta: String,
    },
    /// An invocation of a custom (non-builtin, non-MCP) tool.
    CustomToolUse {
        /// Identifier correlating this tool use to its eventual result.
        tool_use_id: String,
        /// Name of the custom tool being invoked.
        name: String,
        /// The input arguments passed to the tool.
        input: serde_json::Value,
    },
    /// An invocation of a provider-hosted builtin tool.
    BuiltinToolUse {
        /// Identifier correlating this tool use to its eventual result.
        tool_use_id: String,
        /// Name of the builtin tool being invoked.
        name: String,
        /// The input arguments passed to the tool.
        input: serde_json::Value,
    },
    /// A transition in the session's run status.
    Status {
        /// The session's new run status.
        status: RunStatus,
    },
    /// An error condition encountered while executing the session.
    Error {
        /// Human-readable description of the error.
        message: String,
        /// Whether the session can continue (e.g. via drain) or is terminally
        /// failed.
        recoverable: bool,
    },
}

/// A normalised session-stream event: an order-comparable [`id`](Event::id), a
/// timestamp, and a [`kind`](Event::kind) from the closed vocabulary.
///
/// `id` is opaque but lexicographically monotonic within a session, so it serves
/// both as the `listSessionEvents` pagination cursor and as the ordering a
/// conformant adapter must preserve. On the wire the `kind` is flattened, so an
/// event serialises as `{ "id", "at", "type", … }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Opaque, order-comparable identifier within the session's event stream.
    pub id: String,

    /// When the event was emitted.
    pub at: DateTime<Utc>,

    /// The event payload.
    #[serde(flatten)]
    pub kind: EventKind,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn event_flattens_kind_over_type() {
        let ev = Event {
            id: "00000000000000000003".to_owned(),
            at: Utc.with_ymd_and_hms(2026, 7, 10, 0, 0, 0).unwrap(),
            kind: EventKind::AssistantMessageText {
                message_id: "m1".to_owned(),
                delta: "hi".to_owned(),
            },
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "assistant_message_text");
        assert_eq!(json["message_id"], "m1");
        assert_eq!(json["delta"], "hi");
        assert_eq!(json["id"], "00000000000000000003");
        let back: Event = serde_json::from_value(json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn status_and_error_variants_round_trip() {
        for ev in [
            Event {
                id: "00000000000000000001".to_owned(),
                at: Utc.with_ymd_and_hms(2026, 7, 10, 0, 0, 0).unwrap(),
                kind: EventKind::Status {
                    status: RunStatus::Idle,
                },
            },
            Event {
                id: "00000000000000000002".to_owned(),
                at: Utc.with_ymd_and_hms(2026, 7, 10, 0, 0, 0).unwrap(),
                kind: EventKind::Error {
                    message: "boom".to_owned(),
                    recoverable: false,
                },
            },
        ] {
            let s = serde_json::to_string(&ev).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&s).unwrap(), ev);
        }
    }
}
