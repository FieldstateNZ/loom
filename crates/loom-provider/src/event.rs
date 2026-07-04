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
    /// A new content part has started at `index`, carrying its initial shape.
    ///
    /// Some providers announce a content block before streaming its deltas
    /// (e.g. Anthropic's `content_block_start`), and metadata such as a
    /// tool-use call's `id`/`name` arrives here rather than in the deltas. The
    /// `part` is the block in its initial state (e.g. a [`ToolUse`] whose
    /// `input` is still empty); subsequent [`ContentPartDelta`] events fill it
    /// in, and a [`ContentPartComplete`] carries the finished part.
    ///
    /// [`ToolUse`]: loom_core::ContentPart::ToolUse
    /// [`ContentPartDelta`]: TurnEventKind::ContentPartDelta
    /// [`ContentPartComplete`]: TurnEventKind::ContentPartComplete
    ContentPartStarted {
        /// The zero-based index of the content part being started.
        index: usize,
        /// The content part in its initial, pre-delta state.
        part: ContentPart,
    },
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
    /// The turn has ended, with the provider's stop reason and — when the
    /// provider couples the two in one native event, as Anthropic does in
    /// `message_delta` — a final usage snapshot.
    TurnEnded {
        /// Why generation stopped.
        stop_reason: StopReason,
        /// A final usage snapshot delivered alongside the stop reason when the
        /// provider reports both together. `None` when usage instead arrives via
        /// separate [`Usage`](TurnEventKind::Usage) events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    /// A native provider event with no normalised classification — a keep-alive
    /// (e.g. Anthropic's `ping`) or an event kind Loom does not yet model.
    ///
    /// The verbatim event is always available on [`TurnEvent::raw`]; this
    /// variant simply signals "there is no normalised view — read `raw`", so a
    /// translator never has to drop or misclassify an event it cannot normalise.
    Other {
        /// The provider's native event `type`, when it has one (e.g. `"ping"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        native_type: Option<String>,
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
    /// A chunk of the opaque signature over a thinking block (Anthropic's
    /// `signature_delta`). Accumulate these to reconstruct the final
    /// [`ContentPart::Thinking::signature`](loom_core::ContentPart::Thinking).
    SignatureDelta {
        /// The appended signature fragment.
        signature: String,
    },
    /// A citation appended to a text part (Anthropic's `citations_delta`),
    /// preserved verbatim.
    ///
    /// Accumulate these onto the text part's citations so a streamed cited text
    /// block reassembles identically to the non-streaming
    /// [`ContentPart::Text`](loom_core::ContentPart::Text) it corresponds to.
    Citation {
        /// The provider's native citation object, preserved without
        /// interpretation.
        citation: loom_core::Citation,
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
