//! The normalised streaming envelope, [`TurnEvent`].
//!
//! Loom's fidelity promise applies to streaming too: every emitted event
//! carries **both** a provider-agnostic normalised view ([`TurnEventKind`])
//! **and** the verbatim native provider event JSON ([`TurnEvent::raw`]). A
//! client can consume the normalised level for portability, or drop down to the
//! raw provider level for byte-exact fidelity — without choosing one at the
//! transport layer.

mod content_delta;
mod stop_reason;
mod turn_cost;
mod turn_event;
mod turn_event_kind;

pub use content_delta::ContentDelta;
pub use stop_reason::StopReason;
pub use turn_cost::TurnCost;
pub use turn_event::TurnEvent;
pub use turn_event_kind::TurnEventKind;
