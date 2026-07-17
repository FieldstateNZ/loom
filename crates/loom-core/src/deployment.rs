//! A materialization of an agent definition at a provider.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A materialization of an [`AgentDefinition`](crate::AgentDefinition) at a
/// specific provider and environment.
///
/// Deployments are idempotent by `canonical_hash`: before materialising an
/// agent, a server hashes the definition's canonical form and reuses an
/// existing deployment with a matching hash rather than re-materialising. That
/// short-circuit is the server's responsibility, not the adapter's — the
/// adapter faithfully materialises whatever it is told to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Deployment {
    /// Unique identifier of this deployment.
    pub id: Uuid,

    /// The definition this deployment materialises.
    pub agent_definition_id: Uuid,

    /// The provider the definition was deployed to.
    pub provider: String,

    /// The provider's own identifier for the materialised agent.
    pub provider_agent_id: String,

    /// The environment this deployment was created in.
    pub environment_id: String,

    /// The provider's own version tag for the materialised agent, distinct from
    /// the definition's version pointers.
    pub provider_version: String,

    /// Hash of the definition's canonical form at deploy time — the idempotency
    /// key a server short-circuits on.
    pub canonical_hash: String,
}
