//! Persistence for the versioned pricing model.

use async_trait::async_trait;

use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::model::{ModelPrice, NewModelPrice};

/// Persistence for the versioned pricing model.
// `async_trait` (<= 0.1.89) stamps a bare `#[must_use]` on each async trait
// method while the boxed future it returns is already `#[must_use]`, which
// trips `clippy::double_must_use` on newer toolchains. The attribute is
// macro-generated, so this scoped allow is the narrowest place to quiet it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait PricingStore {
    /// Returns the effective price for `(provider, model)` at instant `at`:
    /// the latest row whose `effective_from` is at or before `at`, or `None`
    /// if no such price is configured.
    async fn get_effective_price(
        &self,
        provider: &str,
        model: &str,
        at: DateTime<Utc>,
    ) -> Result<Option<ModelPrice>>;

    /// Inserts a price version, returning the persisted row.
    ///
    /// Prices are versioned, not overwritten: a genuine price change is a new
    /// row with a later `effective_from`. Re-inserting the exact same
    /// `(provider, model, effective_from)` corrects that one version in place
    /// (idempotent seeding).
    async fn upsert_price(&self, price: NewModelPrice) -> Result<ModelPrice>;
}
