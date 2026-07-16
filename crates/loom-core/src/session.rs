//! A durable provider execution context a conversation rides on.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

    /// When the session was created.
    pub created_at: DateTime<Utc>,
}

impl Session {
    /// Constructs a new [`SessionStatus::Active`] session for a conversation,
    /// with a freshly generated [`Session::id`] and `created_at` set to `now`.
    #[must_use]
    pub fn new(conversation_id: Uuid, tenant_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id,
            tenant_id,
            status: SessionStatus::Active,
            created_at: Utc::now(),
        }
    }
}
