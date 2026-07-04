//! Policy for handling cache hints on models without prompt-caching support.

use serde::{Deserialize, Serialize};

/// How a provider should treat a cache hint on a model that does not declare
/// prompt-caching support.
///
/// Cache hints are advisory, so the default is [`CacheNegotiation::SoftIgnore`]:
/// the hint is stripped (and a warning surfaced via a log and/or response
/// header) rather than failing the request. Callers that would rather learn
/// about the mismatch loudly can opt into [`CacheNegotiation::HardFail`].
///
/// Serialized in `snake_case` (`soft_ignore`, `hard_fail`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CacheNegotiation {
    /// Strip cache hints and continue, surfacing a warning. The default.
    #[default]
    SoftIgnore,
    /// Fail the request with a capability error.
    HardFail,
}
