//! `loom-store` — Loom's PostgreSQL persistence layer.
//!
//! Owns the schema (tenants, virtual keys, provider credentials, conversations,
//! messages, usage events), embedded migrations, and typed store traits so that
//! `loom-server` never writes SQL directly. Every accessor that touches
//! tenant-owned data is scoped to a tenant.
//!
//! # Layout
//!
//! - [`PgStore`] is the PostgreSQL implementation of every store trait over a
//!   shared [`sqlx::PgPool`].
//! - The store traits ([`TenantStore`], [`KeyStore`], [`CredentialStore`],
//!   [`McpServerStore`], [`ConversationStore`], [`UsageStore`], [`PricingStore`],
//!   [`OutboxStore`], [`BudgetStore`]) are the persistence surface the rest of
//!   the workspace depends on.
//! - [`run_migrations`] applies the embedded migration set at startup.
//!
//! Conversation history round-trips the [`loom_core`] domain model through
//! JSONB losslessly: a [`loom_core::Conversation`] persisted and reloaded
//! compares equal to the original, including
//! [`loom_core::ContentPart::ProviderExtension`] payloads.
//!
//! # Offline compilation
//!
//! Queries use `sqlx`'s compile-time-checked macros. A committed `.sqlx/`
//! offline cache lets the crate build with no database available
//! (`SQLX_OFFLINE=true cargo build`); CI never needs a live database.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod model;
mod pg;
mod pricing;
mod store;

pub use error::{Result, StoreError};
pub use model::{
    Budget, BudgetAction, BudgetWindow, KeyBudget, McpServer, ModelPrice, NewMcpServer,
    NewModelPrice, NewProviderCredential, NewTenant, NewUsageEvent, NewVirtualKey, OutboxEntry,
    ProviderCredential, RateLimit, RollupGroup, Tenant, UsageEvent, UsageRollup, UsageRollupRow,
    VirtualKey,
};
pub use pg::PgStore;
pub use pricing::Pricer;
pub use store::{
    BudgetStore, ConversationStore, CredentialStore, KeyStore, McpServerStore, OutboxStore,
    PricingStore, TenantStore, UsageStore,
};

/// Re-export of the domain model persisted by this layer.
pub use loom_core;

/// The outcome of a [`drain_usage_outbox`] pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrainReport {
    /// Entries that were successfully recorded and marked processed.
    pub processed: usize,
    /// Entries whose replay failed again (left pending, attempt count bumped).
    pub failed: usize,
}

/// Reprocesses up to `limit` pending [usage outbox](OutboxStore) entries.
///
/// For each parked event this retries the primary
/// [`UsageStore::record_event`] write; on success the entry is marked
/// processed, on failure its attempt count and last error are recorded and it
/// stays pending for a later pass. The user turn that parked the event has
/// already returned success — this is the deferred settlement path, safe to run
/// from a background task or a test.
///
/// # Errors
///
/// Returns [`StoreError`] only if listing pending entries or updating outbox
/// status itself fails; a failure to *replay* an individual event is counted in
/// [`DrainReport::failed`], not propagated.
pub async fn drain_usage_outbox<S>(store: &S, limit: i64) -> Result<DrainReport>
where
    S: UsageStore + OutboxStore + Sync,
{
    let pending = store.list_pending_outbox(limit).await?;
    let mut report = DrainReport::default();
    for entry in pending {
        match store.record_event(entry.payload).await {
            Ok(_) => {
                store.mark_outbox_processed(entry.id).await?;
                report.processed += 1;
            }
            Err(err) => {
                store.mark_outbox_failed(entry.id, &err.to_string()).await?;
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// Applies the embedded migration set to `pool`, bringing an empty database up
/// to the current schema.
///
/// Migrations are embedded at compile time via [`sqlx::migrate!`], so no
/// migration files need to ship alongside the binary. The operation is
/// idempotent: already-applied migrations are skipped.
///
/// Whether to run migrations on startup is the server's decision.
///
/// # Errors
///
/// Returns [`StoreError::Migration`] if a migration fails to apply.
pub async fn run_migrations(pool: &sqlx::PgPool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
