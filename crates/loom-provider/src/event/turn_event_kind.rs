//! The normalised streaming envelope classification, [`TurnEventKind`].

use loom_core::{ContentPart, Usage};
use serde::{Deserialize, Serialize};

use super::content_delta::ContentDelta;
use super::stop_reason::StopReason;

/// The normalised, provider-agnostic classification of a [`TurnEvent`].
///
/// The enum is `#[non_exhaustive]`: more event kinds will be normalised over
/// time. Match with a wildcard arm and fall back to [`TurnEvent::raw`] for
/// anything not yet modelled.
///
/// [`TurnEvent`]: super::turn_event::TurnEvent
/// [`TurnEvent::raw`]: super::turn_event::TurnEvent::raw
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
    ///
    /// [`TurnEvent::raw`]: super::turn_event::TurnEvent::raw
    Other {
        /// The provider's native event `type`, when it has one (e.g. `"ping"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        native_type: Option<String>,
    },
}
