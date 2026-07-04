//! The streaming envelope, [`TurnEvent`].

use serde::{Deserialize, Serialize};

use super::turn_event_kind::TurnEventKind;

/// A single streaming event: a normalised envelope plus the verbatim native
/// provider event it was derived from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
