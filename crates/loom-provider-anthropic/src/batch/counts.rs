//! [`BatchRequestCounts`]: per-status request counts for a batch.

/// Per-status request counts reported for a batch, mirroring Anthropic's
/// `request_counts` object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatchRequestCounts {
    /// Requests still being processed.
    pub processing: i64,
    /// Requests that completed successfully.
    pub succeeded: i64,
    /// Requests that errored.
    pub errored: i64,
    /// Requests that were canceled.
    pub canceled: i64,
    /// Requests that expired.
    pub expired: i64,
}
