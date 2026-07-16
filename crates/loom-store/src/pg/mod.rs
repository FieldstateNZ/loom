//! PostgreSQL implementation of the store traits.

mod batch;
mod budget;
mod conversation;
mod credential;
mod key;
mod mcp;
mod outbox;
mod pricing;
mod session;
mod tenant;
mod usage;

use rust_decimal::Decimal;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::error::Result;
use crate::model::{Budget, BudgetAction, BudgetWindow};

/// A PostgreSQL-backed store implementing every store trait over a shared
/// connection pool.
///
/// Clone is cheap — the pool is reference-counted — so a single `PgStore` can
/// be shared across request handlers.
#[derive(Clone, Debug)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    /// Wraps an existing connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Connects to the database at `url`, building a default pool.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`](crate::StoreError::Database) if a
    /// connection cannot be established.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new().connect(url).await?;
        Ok(Self::new(pool))
    }

    /// Borrows the underlying connection pool.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// Assembles a [`Budget`] from its three nullable stored columns, treating a
/// present `limit_amount` as the presence signal. Unknown window/action text
/// falls back to the safe defaults (`Total` window, `Block` action) so a corrupt
/// row never silently disables enforcement.
///
/// Shared by the [`KeyStore`](crate::KeyStore) and
/// [`BudgetStore`](crate::BudgetStore) row-mapping in sibling modules.
fn build_budget(
    limit_amount: Option<Decimal>,
    window: Option<String>,
    action: Option<String>,
) -> Option<Budget> {
    limit_amount.map(|limit_amount| Budget {
        limit_amount,
        window: window
            .as_deref()
            .and_then(BudgetWindow::parse)
            .unwrap_or(BudgetWindow::Total),
        action: action
            .as_deref()
            .and_then(BudgetAction::parse)
            .unwrap_or(BudgetAction::Block),
    })
}
