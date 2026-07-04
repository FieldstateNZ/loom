//! The normalised streaming envelope, [`TurnEvent`].
//!
//! Loom's fidelity promise applies to streaming too: every emitted event
//! carries **both** a provider-agnostic normalised view ([`TurnEventKind`])
//! **and** the verbatim native provider event JSON ([`TurnEvent::raw`]). A
//! client can consume the normalised level for portability, or drop down to the
//! raw provider level for byte-exact fidelity — without choosing one at the
//! transport layer.

use loom_core::{ContentPart, Usage};
use serde::{Deserialize, Serialize};

/// A single streaming event: a normalised envelope plus the verbatim native
/// provider event it was derived from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnEvent {
    /// The normalised, provider-agnostic view of this event.
    pub kind: TurnEventKind,
    /// The verbatim native provider event JSON this envelope was derived from.
    ///
    /// This is never lossy: whatever the provider sent on the wire is preserved
    /// here so clients that need byte-exact fidelity can bypass the normalised
    /// view entirely.
    pub raw: serde_json::Value,
}

impl TurnEvent {
    /// Builds a `TurnEvent` from a normalised `kind` and its originating native
    /// provider event `raw`.
    pub fn new(kind: TurnEventKind, raw: serde_json::Value) -> Self {
        Self { kind, raw }
    }
}

/// The normalised, provider-agnostic classification of a [`TurnEvent`].
///
/// The enum is `#[non_exhaustive]`: more event kinds will be normalised over
/// time. Match with a wildcard arm and fall back to [`TurnEvent::raw`] for
/// anything not yet modelled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnEventKind {
    /// The turn has begun. Emitted once, first.
    TurnStarted,
    /// An incremental delta to the content part at `index`.
    ContentPartDelta {
        /// The zero-based index of the content part being appended to.
        index: usize,
        /// The incremental change.
        delta: ContentDelta,
    },
    /// The content part at `index` is now complete. Carries the fully
    /// assembled [`ContentPart`].
    ContentPartComplete {
        /// The zero-based index of the completed content part.
        index: usize,
        /// The finished content part.
        part: ContentPart,
    },
    /// A usage update. Providers may emit this incrementally and/or once at the
    /// end of the turn.
    Usage(Usage),
    /// The turn has ended, with the provider's stop reason.
    TurnEnded {
        /// Why generation stopped.
        stop_reason: StopReason,
    },
}

/// An incremental change to a streaming content part.
///
/// The enum is `#[non_exhaustive]`; match with a wildcard arm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentDelta {
    /// A chunk of text appended to a text part.
    Text {
        /// The appended text fragment.
        text: String,
    },
    /// A chunk of partial JSON appended to a tool-use input, streamed as it is
    /// generated (not yet parseable on its own).
    Json {
        /// The appended partial-JSON fragment.
        partial_json: String,
    },
    /// A chunk of reasoning text appended to a thinking part.
    Thinking {
        /// The appended reasoning fragment.
        thinking: String,
    },
}

/// The reason a provider stopped generating a turn.
///
/// Known reasons are modelled as typed variants; anything a provider reports
/// that Loom does not (yet) model is preserved verbatim in
/// [`StopReason::Other`]. The enum is `#[non_exhaustive]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StopReason {
    /// The model reached a natural end of its turn.
    EndTurn,
    /// The maximum token budget was reached.
    MaxTokens,
    /// A configured stop sequence was generated.
    StopSequence,
    /// The model paused to invoke a tool.
    ToolUse,
    /// The model paused a long-running turn (e.g. a server-side tool) and can
    /// be resumed.
    PauseTurn,
    /// The model declined to continue.
    Refusal,
    /// A provider-specific stop reason Loom does not model as a typed variant,
    /// preserved verbatim.
    Other(String),
}
