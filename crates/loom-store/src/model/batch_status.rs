//! The lifecycle status of a batch job.

use serde::{Deserialize, Serialize};

/// The lifecycle status of a [`BatchJob`](crate::BatchJob).
///
/// The happy path is [`Created`](Self::Created) →
/// [`Submitting`](Self::Submitting) → [`InProgress`](Self::InProgress) →
/// [`Ended`](Self::Ended); a cancel request moves an in-flight job through
/// [`Canceling`](Self::Canceling) before it settles at `Ended`.
///
/// [`Submitting`](Self::Submitting) is a short-lived **claim** state: the poll
/// worker flips `created → submitting` atomically before it calls the provider,
/// so exactly one worker (even across replicas) ever submits a given job. See
/// [`BatchStore::claim_batch_for_submission`](crate::BatchStore::claim_batch_for_submission).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    /// Accepted and persisted, not yet submitted to the provider.
    Created,
    /// Claimed by the poll worker for submission: the `created → submitting`
    /// transition is an atomic claim, so a concurrent worker cannot also submit
    /// the same job. Left only by a guarded `mark_batch_submitted`
    /// (`→ in_progress`) or `release_batch_submission` (`→ created`, on a submit
    /// failure).
    Submitting,
    /// Submitted to the provider and being processed.
    InProgress,
    /// A cancellation has been requested; the provider is winding the batch
    /// down.
    Canceling,
    /// Terminal: every item has a result (succeeded, errored, canceled or
    /// expired).
    Ended,
}

impl BatchStatus {
    /// The stored text form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Submitting => "submitting",
            Self::InProgress => "in_progress",
            Self::Canceling => "canceling",
            Self::Ended => "ended",
        }
    }

    /// Parses the stored text form, or `None` if it is not a known status.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "created" => Some(Self::Created),
            "submitting" => Some(Self::Submitting),
            "in_progress" => Some(Self::InProgress),
            "canceling" => Some(Self::Canceling),
            "ended" => Some(Self::Ended),
            _ => None,
        }
    }

    /// Whether the job is still advancing (a poll pass should visit it).
    #[must_use]
    pub fn is_active(self) -> bool {
        !matches!(self, Self::Ended)
    }
}
