//! The computed cost of a unit of usage, [`Cost`].

use rust_decimal::Decimal;

/// The computed cost of a unit of usage.
///
/// This is intentionally minimal: it is the value returned by the pricing
/// **hook** [`Provider::count_cost`](crate::Provider::count_cost). The pricing
/// *data* (per-model, per-token rates) is out of scope here and lands with spend
/// tracking. The optional input/output breakdown lets a provider attribute cost
/// to prompt versus completion tokens when it has the rates to do so.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Cost {
    /// ISO 4217 currency code (e.g. `"USD"`). Empty only for a [`Default`]
    /// value; the [`Cost::zero`] placeholder constructor always sets a currency.
    pub currency: String,
    /// The total monetary amount.
    pub amount: Decimal,
    /// The portion of `amount` attributable to input (prompt) tokens, if known.
    pub input_amount: Option<Decimal>,
    /// The portion of `amount` attributable to output (completion) tokens, if
    /// known.
    pub output_amount: Option<Decimal>,
}

impl Cost {
    /// A zero cost in the given `currency`. Useful as a placeholder for a
    /// pricing hook that has no rate data yet.
    #[must_use]
    pub fn zero(currency: impl Into<String>) -> Self {
        Self {
            currency: currency.into(),
            amount: Decimal::ZERO,
            input_amount: None,
            output_amount: None,
        }
    }
}
