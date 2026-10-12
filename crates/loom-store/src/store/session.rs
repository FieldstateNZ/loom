//! Persistence for sessions — the durable execution contexts a conversation
//! rides on.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use loom_core::Session;

/// Read access to [`Session`]s, scoped to a tenant.
///
/// A conversation's active session is created implicitly by
/// [`ConversationStore::create_conversation`](crate::ConversationStore::create_conversation)
/// (and, in later slices, by migration). This trait exposes the read side so a
/// conversation's execution contexts are addressable.
// `async_trait` (<= 0.1.89) stamps a bare `#[must_use]` on each async trait
// method while the boxed future it returns is already `#[must_use]`, which
// trips `clippy::double_must_use` on newer toolchains. The attribute is
// macro-generated, so this scoped allow is the narrowest place to quiet it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait SessionStore {
    /// Loads a session by id, scoped to a tenant. Returns `None` if it does not
    /// exist or belongs to another tenant.
    async fn get_session(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<Session>>;

    /// Lists a conversation's sessions, oldest-first, scoped to a tenant.
    ///
    /// Returns an empty vector if the conversation does not exist or belongs to
    /// another tenant.
    async fn list_sessions(&self, tenant_id: Uuid, conversation_id: Uuid) -> Result<Vec<Session>>;
}
