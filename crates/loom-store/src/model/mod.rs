//! Row and input types exchanged with the store traits.
//!
//! These are plain data types: the "New*" structs describe an insertion, and
//! the remaining structs mirror a persisted row. Conversation history is not
//! modelled here — it round-trips the [`loom_core`] domain model directly.

mod agent;
mod batch_item;
mod batch_job;
mod batch_status;
mod budget;
mod credential;
mod key;
mod mcp;
mod outbox;
mod pending_tool_call;
mod pricing;
mod rate_limit;
mod tenant;
mod usage_event;
mod usage_rollup;

pub use agent::NewAgentDefinition;
pub use batch_item::{BatchItem, BatchItemStatus, NewBatchItem};
pub use batch_job::{BatchCounts, BatchJob, NewBatchJob};
pub use batch_status::BatchStatus;
pub use budget::{Budget, BudgetAction, BudgetWindow, KeyBudget};
pub use credential::{NewProviderCredential, ProviderCredential};
pub use key::{NewVirtualKey, VirtualKey};
pub use mcp::{McpServer, NewMcpServer};
pub use outbox::OutboxEntry;
pub use pending_tool_call::NewPendingToolCall;
pub use pricing::{ModelPrice, NewModelPrice};
pub use rate_limit::RateLimit;
pub use tenant::{NewTenant, Tenant};
pub use usage_event::{NewUsageEvent, UsageEvent};
pub use usage_rollup::{RollupGroup, UsageRollup, UsageRollupRow};
