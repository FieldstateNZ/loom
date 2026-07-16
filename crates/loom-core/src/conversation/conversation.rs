//! A persisted, multi-tenant conversation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ProviderBinding;
use crate::{CacheHint, Message};

/// A persisted, multi-tenant conversation: its identity, provider binding,
/// message history, and metadata.
///
/// This is the top-level aggregate of the domain model. It owns the ordered
/// [`Message`] history but not the request-time [`ConversationOptions`], which
/// are supplied per request by higher layers.
///
/// [`ConversationOptions`]: crate::ConversationOptions
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Conversation {
    /// The conversation's unique identifier.
    pub id: Uuid,

    /// The tenant that owns this conversation, for multi-tenant isolation and
    /// attribution.
    pub tenant_id: Uuid,

    /// The provider and model this conversation is bound to.
    pub binding: ProviderBinding,

    /// An optional system prompt applied to the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,

    /// An optional prompt-cache breakpoint on the system prompt.
    ///
    /// When set (and [`system`](Conversation::system) is present), provider
    /// translators mark the system prefix — which, together with any tools,
    /// forms the stable head of the request — as a cache breakpoint. This is a
    /// request-render concern rather than durable state: it is **not**
    /// persisted, so it takes effect only on the in-memory conversation a turn
    /// is run against (the stateless turn path, or the auto-cache strategy for
    /// persisted history). It is absent (rather than `null`) when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_cache: Option<CacheHint>,

    /// The ordered message history.
    #[serde(default)]
    pub messages: Vec<Message>,

    /// The conversation's current (active) [`Session`]: the execution context
    /// new turns run against.
    ///
    /// OASP models a Conversation as a durable thread riding a lineage of
    /// disposable Sessions; this is the live one. It is always populated for a
    /// live conversation — a conversation with no current session is only a
    /// transient mid-migration state — and is absent (rather than `null`) when
    /// unset.
    ///
    /// [`Session`]: crate::Session
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_session_id: Option<Uuid>,

    /// The ids of this conversation's superseded [`Session`]s, oldest-first —
    /// its lineage.
    ///
    /// Append-only: a migration appends the outgoing session id here as it swaps
    /// in a fresh one. Empty until the conversation has migrated at least once.
    ///
    /// [`Session`]: crate::Session
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_session_ids: Vec<Uuid>,

    /// Free-form, caller-supplied metadata (tags, correlation IDs, …).
    ///
    /// Defaults to JSON `null` when unset.
    #[serde(default)]
    pub metadata: serde_json::Value,

    /// When the conversation was created.
    pub created_at: DateTime<Utc>,

    /// When the conversation was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    /// Constructs a new, empty conversation with a freshly generated
    /// [`Conversation::id`], a freshly minted active
    /// [`current_session_id`](Conversation::current_session_id), and `created_at`
    /// / `updated_at` set to `now`.
    ///
    /// The message history is empty, the lineage is empty, there is no system
    /// prompt, and [`Conversation::metadata`] is JSON `null`.
    #[must_use]
    pub fn new(tenant_id: Uuid, binding: ProviderBinding) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            tenant_id,
            binding,
            system: None,
            system_cache: None,
            messages: Vec::new(),
            current_session_id: Some(Uuid::new_v4()),
            previous_session_ids: Vec::new(),
            metadata: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
        }
    }
}
