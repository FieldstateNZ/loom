//! A spend budget attachable at the tenant or the key level.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A spend budget attachable at the tenant or the key level.
///
/// All three fields are stored together: a scope either has a complete budget or
/// none at all. A key-level budget overrides its tenant's default (see
/// [`BudgetStore`](crate::BudgetStore)).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    /// The spend limit, in the gateway's accounting currency.
    pub limit_amount: Decimal,
    /// The rolling window the limit applies over.
    pub window: BudgetWindow,
    /// What to do when the limit is reached.
    pub action: BudgetAction,
}

/// The rolling window a [`Budget`] limit applies over.
///
/// [`Daily`](Self::Daily), [`Weekly`](Self::Weekly) and
/// [`Monthly`](Self::Monthly) are rolling look-back windows;
/// [`Total`](Self::Total) is all-time (no lower bound).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetWindow {
    /// The trailing 24 hours.
    Daily,
    /// The trailing 7 days.
    Weekly,
    /// The trailing 30 days.
    Monthly,
    /// All recorded usage (no lower time bound).
    Total,
}

impl BudgetWindow {
    /// The stored text form (`"daily"`, `"weekly"`, `"monthly"`, `"total"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
            Self::Total => "total",
        }
    }

    /// Parses the stored text form, or `None` if it is not a known window.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            "total" => Some(Self::Total),
            _ => None,
        }
    }

    /// The inclusive lower bound of the window relative to `now`, or `None` for
    /// [`Total`](Self::Total) (an open lower bound — all recorded usage).
    #[must_use]
    pub fn start(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Daily => Some(now - chrono::Duration::days(1)),
            Self::Weekly => Some(now - chrono::Duration::weeks(1)),
            Self::Monthly => Some(now - chrono::Duration::days(30)),
            Self::Total => None,
        }
    }
}

/// What to do when a [`Budget`] limit is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetAction {
    /// Reject further spend with a `402` structured error.
    Block,
    /// Allow the request but flag it (a warning header and a logged event).
    Warn,
}

impl BudgetAction {
    /// The stored text form (`"block"`, `"warn"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Warn => "warn",
        }
    }

    /// Parses the stored text form, or `None` if it is not a known action.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "block" => Some(Self::Block),
            "warn" => Some(Self::Warn),
            _ => None,
        }
    }
}

/// Backwards-compatible alias for the pre-#10 name of [`Budget`].
pub type KeyBudget = Budget;
