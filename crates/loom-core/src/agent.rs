//! Named, versioned, provider-neutral agent definitions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The immutable content of one [`AgentDefinitionVersion`] — the snapshot of
/// what an agent *is* at a given version.
///
/// This elevates what a conversation configures ad hoc today (provider, model,
/// system prompt) into a named, versioned definition. The tool configuration is
/// carried as an extensible JSON value for now; a typed tool vocabulary
/// (`builtin` / `custom` / `mcp`) is a later refinement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentContent {
    /// The provider this version targets.
    pub provider: String,

    /// The model this version targets.
    pub model: String,

    /// The agent's instructions (its system prompt), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,

    /// The agent's tool configuration. Extensible; defaults to JSON `null`.
    #[serde(default)]
    pub tools: serde_json::Value,
}

impl AgentContent {
    /// Constructs minimal content binding a provider and model, with no
    /// instructions and no tools.
    #[must_use]
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            instructions: None,
            tools: serde_json::Value::Null,
        }
    }
}

/// A provider-neutral, named, versioned agent.
///
/// An `AgentDefinition` carries two version pointers into its immutable
/// [`AgentDefinitionVersion`] snapshots:
///
/// - [`draft_version`](AgentDefinition::draft_version) advances on every edit.
/// - [`published_version`](AgentDefinition::published_version) is advanced
///   **only** by `publish` — monotonically, with no unpublish.
///
/// A conversation pins the *published* version (version pinning attaches here as
/// later slices land); the draft is what a builder edits against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentDefinition {
    /// The definition's unique identifier.
    pub id: Uuid,

    /// The tenant that owns this definition, for multi-tenant isolation.
    pub tenant_id: Uuid,

    /// The definition's human-readable name.
    pub name: String,

    /// The current draft version — advances on every edit. Starts at `1`.
    pub draft_version: i32,

    /// The current published version, advanced only by `publish`. Absent
    /// (rather than `null`) until the definition has been published at least
    /// once; a conversation cannot pin a never-published definition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_version: Option<i32>,

    /// When the definition was created.
    pub created_at: DateTime<Utc>,

    /// When the definition was last updated (an edit or a publish).
    pub updated_at: DateTime<Utc>,
}

/// An immutable content snapshot of an [`AgentDefinition`] at one version.
///
/// Recorded at mint time whether or not the version is ever published, so the
/// exact content that produced a given turn is always resolvable — not merely an
/// integer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentDefinitionVersion {
    /// The definition this snapshot belongs to.
    pub agent_definition_id: Uuid,

    /// The version number this snapshot records.
    pub version: i32,

    /// The immutable content at this version.
    pub content: AgentContent,

    /// When this version was minted.
    pub created_at: DateTime<Utc>,
}
