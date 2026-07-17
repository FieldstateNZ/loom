//! A durable provider execution context a conversation rides on.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AgentVersionRef;

/// A resource mounted into a [`Session`] at creation: a file, an opaque memory
/// store, or a GitHub repository. Discriminated on `type` on the wire.
///
/// Resources are mounted in full at session creation and are **not**
/// automatically carried forward to later sessions; re-mounting them for a new
/// version is `migrate`'s job, not something a session does on its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionResource {
    /// A single mounted file.
    File {
        /// Identifier of the mounted file.
        file_id: String,
    },
    /// An opaque mounted memory store (v0 treats memory as opaque).
    MemoryStore {
        /// Opaque identifier of the mounted memory store.
        store_id: String,
    },
    /// A mounted GitHub repository.
    #[serde(rename = "github_repository")]
    GitHubRepository {
        /// Owner (user or organization) of the repository.
        owner: String,
        /// Name of the repository.
        repo: String,
        /// Branch, tag, or commit SHA to mount; the provider's default branch
        /// when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        r#ref: Option<String>,
    },
}

/// The lifecycle state of a [`Session`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionStatus {
    /// The conversation's live session — the one new turns run against. A
    /// conversation has exactly one active session at a time.
    Active,
    /// A session that has been superseded by a later one and now survives only
    /// as history in the conversation's lineage. (Sessions become superseded
    /// when a conversation migrates onto a fresh session; migration lands in a
    /// later slice.)
    Superseded,
}

/// A durable provider execution context a
/// [`Conversation`](crate::Conversation) rides on.
///
/// OASP's structural insight is that a *Conversation is not a Session*: the
/// durable thread a user cares about outlives the disposable provider execution
/// contexts it runs on. A `Session` is one such context. A conversation always
/// has exactly one **active** session — its
/// [`current_session_id`](crate::Conversation::current_session_id) — and keeps
/// the ids of any superseded sessions as its
/// [`previous_session_ids`](crate::Conversation::previous_session_ids) lineage.
///
/// This is the substrate the managed-agent contract is built on: version
/// pinning, mounted resources and vault bindings attach to a session as later
/// slices land.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Session {
    /// The session's unique identifier.
    pub id: Uuid,

    /// The conversation this session belongs to.
    pub conversation_id: Uuid,

    /// The tenant that owns this session, for multi-tenant isolation and
    /// attribution.
    pub tenant_id: Uuid,

    /// The session's lifecycle state.
    pub status: SessionStatus,

    /// The immutable agent version this session is pinned to, once created
    /// against a published agent. `None` until version pinning is wired at
    /// `createConversation`/`createSession`; omitted from the serialized form
    /// when unset (a JSON `null` on input also deserialises to `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_agent_version: Option<AgentVersionRef>,

    /// Resources mounted into the session at creation, in full. Empty until
    /// session creation mounts them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<SessionResource>,

    /// Credential vault ids attached at creation, matched to MCP servers by URL.
    /// Empty until session creation attaches them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vault_ids: Vec<String>,

    /// When the session was created.
    pub created_at: DateTime<Utc>,
}

impl Session {
    /// Constructs a new [`SessionStatus::Active`] session for a conversation,
    /// with a freshly generated [`Session::id`] and `created_at` set to `now`.
    ///
    /// The pin, resources and vaults are empty; they are populated when session
    /// creation pins a version and mounts resources/vaults.
    #[must_use]
    pub fn new(conversation_id: Uuid, tenant_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id,
            tenant_id,
            status: SessionStatus::Active,
            pinned_agent_version: None,
            resources: Vec::new(),
            vault_ids: Vec::new(),
            created_at: Utc::now(),
        }
    }
}
