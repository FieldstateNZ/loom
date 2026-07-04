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
//!   [`ConversationStore`], [`UsageStore`]) are the persistence surface the
//!   rest of the workspace depends on.
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
mod store;

pub use error::{Result, StoreError};
pub use model::{
    KeyBudget, NewProviderCredential, NewTenant, NewUsageEvent, NewVirtualKey, ProviderCredential,
    Tenant, UsageEvent, UsageRollup, VirtualKey,
};
pub use pg::PgStore;
pub use store::{ConversationStore, CredentialStore, KeyStore, TenantStore, UsageStore};

/// Re-export of the domain model persisted by this layer.
pub use loom_core;

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
