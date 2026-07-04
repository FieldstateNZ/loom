//! Why a provider stopped generating a turn, [`StopReason`].

use serde::{Deserialize, Serialize};

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
