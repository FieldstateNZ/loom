//! Structured provider errors and the pricing-hook cost type.

use rust_decimal::Decimal;
use thiserror::Error;

use crate::capability::Capability;

/// The error type returned by [`Provider`](crate::Provider) operations.
///
/// Errors are structured so callers can react programmatically rather than
/// parse strings. Where a provider returns a native error body, it is preserved
/// verbatim in [`ProviderError::Api::payload`] so no fidelity is lost.
///
/// The enum is `#[non_exhaustive]`; match with a wildcard arm.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProviderError {
    /// A required capability is not supported by the bound model. Raised by
    /// capability negotiation before any request is dispatched.
    #[error("capability {capability:?} is not supported by {provider}/{model}")]
    CapabilityUnsupported {
        /// The capability the request required.
        capability: Capability,
        /// The provider whose model was bound.
        provider: String,
        /// The model that lacks the capability.
        model: String,
    },

    /// The requested model is not known to the provider.
    #[error("model {model} is not offered by provider {provider}")]
    ModelNotFound {
        /// The provider that was queried.
        provider: String,
        /// The model identifier that could not be found.
        model: String,
    },

    /// The provider's API returned an error response. The native error body is
    /// preserved in `payload`.
    #[error("provider api error{}: {message}", .status.map(|s| format!(" (status {s})")).unwrap_or_default())]
    Api {
        /// The HTTP status code, if the error originated from an HTTP response.
        status: Option<u16>,
        /// A human-readable summary of the error.
        message: String,
        /// The verbatim provider-native error payload, if any.
        payload: Option<serde_json::Value>,
    },

    /// A transport-level failure (connection, timeout, TLS, …).
    #[error("transport error: {0}")]
    Transport(String),

    /// A request or response could not be serialised or deserialised.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Any other provider error that does not fit the categories above.
    #[error("provider error: {0}")]
    Other(String),
}

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
