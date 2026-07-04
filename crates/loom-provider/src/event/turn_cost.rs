//! Loom's authoritative priced cost for a turn, [`TurnCost`].

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Loom's authoritative priced cost for a turn, computed at turn time from the
/// gateway's pricing table.
///
/// This is distinct from the provider-hook [`Cost`](crate::Cost): that type is
/// the provider's own estimate (not [`Serialize`]), whereas `TurnCost` is the
/// gateway's own number, priced from its `(provider, model)` rate table via
/// `loom-store`'s `Pricer`. It rides on the non-streaming turn response and the
/// streaming `turn_ended` event, computed once per turn and never
/// double-priced. `None` on the turn response/event when no price is
/// configured for the (provider, model) — a pricing miss never fails the turn.
///
/// `/v1/usage` remains the asynchronous, eventually-consistent aggregate
/// (written best-effort through an outbox); `TurnCost` is the immediate,
/// authoritative per-turn figure computed inline at turn time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TurnCost {
    /// The total monetary amount (serialized as a decimal string, matching the
    /// usage-rollup cost fields).
    pub amount: Decimal,
    /// ISO 4217 currency code — `"USD"` (the pricing table's currency).
    pub currency: String,
}
