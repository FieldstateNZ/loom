//! `loom-store` — Loom's PostgreSQL persistence layer.
//!
//! Owns the schema (tenants, virtual keys, provider credentials, conversations,
//! messages, usage events), embedded migrations, and typed store traits so that
//! `loom-server` never writes SQL directly. Every query is tenant-scoped.
//!
//! > **Scaffold.** Schema, migrations and store traits land in issue #6.
#![forbid(unsafe_code)]

/// Re-export of the domain model persisted by this layer.
pub use loom_core;
