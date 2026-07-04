//! The non-streaming turn response envelope, [`TurnResponse`].

use serde::Serialize;
use utoipa::ToSchema;

use loom_core::Message;
use loom_provider::TurnCost;

/// The `application/json` body returned by a non-streaming turn
/// (`POST /v1/conversations/{id}/turns`, `POST /v1/turns`).
///
/// `cost` is Loom's authoritative priced cost for this turn — computed once,
/// inline, from the effective price for the turn's `(provider, model)` at turn
/// time, not `GET /v1/usage`'s eventually-consistent, asynchronously drained
/// aggregate. It is `None` only when no price is configured for the
/// (provider, model); a pricing miss never fails the turn. The streaming
/// counterpart (`stream=true`) carries the identical value on the terminal
/// `turn_ended` [`TurnEvent`](loom_provider::TurnEvent)'s `cost` field, so a
/// caller sees the same number whether it streams or not.
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct TurnResponse {
    /// The assistant's turn.
    pub(super) message: Message,
    /// Loom's authoritative priced cost for this turn, or `None` when no price
    /// is configured for the (provider, model).
    pub(super) cost: Option<TurnCost>,
}
