//! Persistence for tenant-scoped MCP server registrations.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::model::{McpServer, NewMcpServer};

/// Persistence for tenant-scoped MCP server registrations.
///
/// Conversations reference a server by name; the resolver loads the row,
/// decrypts its authorization token, and injects it into the provider request
/// server-side. Every accessor is scoped to a `tenant_id` so one tenant can
/// never read or delete another tenant's registration.
#[async_trait]
pub trait McpServerStore {
    /// Inserts a registration, or replaces the existing one for the same
    /// `(tenant_id, name)` pair. Returns the persisted row.
    async fn upsert_mcp_server(&self, new: NewMcpServer) -> Result<McpServer>;

    /// Fetches a tenant's registration by name, or `None` if absent.
    async fn get_mcp_server(&self, tenant_id: Uuid, name: &str) -> Result<Option<McpServer>>;

    /// Lists a tenant's registrations, ordered by name.
    async fn list_mcp_servers(&self, tenant_id: Uuid) -> Result<Vec<McpServer>>;

    /// Deletes a tenant's registration by name. Returns `true` if one was
    /// deleted.
    async fn delete_mcp_server(&self, tenant_id: Uuid, name: &str) -> Result<bool>;
}
