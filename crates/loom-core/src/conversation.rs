//! Conversations and their binding to a provider.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{CacheHint, Message};

/// The provider a conversation is bound to, and the model to use.
///
/// The binding is intentionally a pair of plain strings rather than an
/// enumeration: providers and their model catalogues evolve independently of
/// Loom's release cycle, and Loom must never reject a valid model just because
/// it predates a given build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBinding {
    /// The provider's name (e.g. `"anthropic"`).
    pub provider: String,
    /// The model identifier as the provider expects it (e.g.
    /// `"claude-opus-4-8"`).
    pub model: String,
}

impl ProviderBinding {
    /// Constructs a binding from a provider name and model identifier.
    #[must_use]
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

/// A persisted, multi-tenant conversation: its identity, provider binding,
/// message history, and metadata.
///
/// This is the top-level aggregate of the domain model. It owns the ordered
/// [`Message`] history but not the request-time [`ConversationOptions`], which
/// are supplied per request by higher layers.
///
/// [`ConversationOptions`]: crate::ConversationOptions
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// [`Conversation::id`] and `created_at` / `updated_at` set to `now`.
    ///
    /// The message history is empty, there is no system prompt, and
    /// [`Conversation::metadata`] is JSON `null`.
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
            metadata: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
        }
    }
}
