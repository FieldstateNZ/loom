//! Typed, tenant-scoped store traits.
//!
//! These traits are the only persistence surface `loom-server` depends on, so
//! it never writes SQL. Every accessor that touches tenant-owned data takes a
//! `tenant_id` and scopes its query to it.

mod agent;
mod batch;
mod budget;
mod conversation;
mod credential;
mod key;
mod mcp;
mod outbox;
mod pending_tool_call;
mod pricing;
mod session;
mod session_event;
mod tenant;
mod usage;

pub use agent::AgentStore;
pub use batch::BatchStore;
pub use budget::BudgetStore;
pub use conversation::ConversationStore;
pub use credential::CredentialStore;
pub use key::KeyStore;
pub use mcp::McpServerStore;
pub use outbox::OutboxStore;
pub use pending_tool_call::{PendingToolCallStore, ResolveOutcome};
pub use pricing::PricingStore;
pub use session::SessionStore;
pub use session_event::SessionEventStore;
pub use tenant::TenantStore;
pub use usage::UsageStore;
