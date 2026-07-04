//! A usage event parked in the outbox because its primary write did not
//! complete.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::usage_event::NewUsageEvent;

/// A usage event parked in the outbox because its primary write did not
/// complete.
///
/// The full [`NewUsageEvent`] is preserved verbatim in
/// [`payload`](Self::payload) so a drain pass can replay it unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct OutboxEntry {
    /// The outbox row's unique identifier.
    pub id: Uuid,
    /// The parked usage event, exactly as it would have been recorded.
    pub payload: NewUsageEvent,
    /// Lifecycle status: `"pending"` or `"processed"`.
    pub status: String,
    /// How many drain attempts have been made.
    pub attempts: i32,
    /// The last error observed while draining, if any.
    pub last_error: Option<String>,
    /// When the entry was parked.
    pub created_at: DateTime<Utc>,
}
