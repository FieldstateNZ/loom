//! Per-key request/token rate limits.

use serde::{Deserialize, Serialize};

/// Per-key request/token rate limits, enforced by an in-process token bucket.
///
/// Either dimension may be `None` (unlimited). Single-instance for v1;
/// distributed limiting across replicas is deferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimit {
    /// Maximum requests per minute, or `None` for unlimited.
    pub requests_per_min: Option<i64>,
    /// Maximum tokens per minute, or `None` for unlimited.
    pub tokens_per_min: Option<i64>,
}

impl RateLimit {
    /// Whether this limit constrains anything (at least one dimension set).
    #[must_use]
    pub fn is_some(&self) -> bool {
        self.requests_per_min.is_some() || self.tokens_per_min.is_some()
    }
}
