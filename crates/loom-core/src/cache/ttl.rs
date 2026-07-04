//! Cache time-to-live tiers.

use serde::{Deserialize, Serialize};

/// How long a provider should keep a cache entry alive.
///
/// Anthropic exposes exactly these two ephemeral time-to-live tiers; the names
/// are provider-agnostic so other providers can map onto them. Serialized in
/// `snake_case` (`five_minutes`, `one_hour`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CacheTtl {
    /// A short-lived cache entry — Anthropic's default five-minute tier.
    FiveMinutes,
    /// A longer-lived cache entry — Anthropic's one-hour tier.
    OneHour,
}
