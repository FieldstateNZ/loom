//! Typed, tenant-scoped store traits.
//!
//! These traits are the only persistence surface `loom-server` depends on, so
//! it never writes SQL. Every accessor that touches tenant-owned data takes a
//! `tenant_id` and scopes its query to it.

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

pub use batch::BatchStore;
pub use budget::BudgetStore;
pub use conversation::ConversationStore;
pub use credential::CredentialStore;
pub use key::KeyStore;
pub use mcp::McpServerStore;
pub use outbox::OutboxStore;
pub use pricing::PricingStore;
pub use session::SessionStore;
pub use tenant::TenantStore;
pub use usage::UsageStore;
