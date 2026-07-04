//! Persistence for conversations and their message history.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use loom_core::{Conversation, Message};

/// Persistence for conversations and their message history.
///
/// History round-trips the loom-core domain model through JSONB losslessly:
/// a conversation persisted and reloaded compares equal to the original.
#[async_trait]
pub trait ConversationStore {
    /// Persists a conversation together with its current message history.
    async fn create_conversation(&self, conversation: &Conversation) -> Result<()>;

    /// Loads a conversation (with its full ordered history) by id, scoped to a
    /// tenant. Returns `None` if it does not exist or belongs to another
    /// tenant.
    async fn get_conversation(&self, tenant_id: Uuid, id: Uuid) -> Result<Option<Conversation>>;

    /// Loads a page of a conversation's messages, ordered by sequence, scoped
    /// to a tenant.
    ///
    /// `limit` caps the number of messages returned and `offset` skips that
    /// many from the start of the history. Returns an empty vector if the
    /// conversation does not exist or belongs to another tenant.
    async fn list_messages(
        &self,
        tenant_id: Uuid,
        conversation_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Message>>;

    /// Appends a message to a conversation and bumps its `updated_at`, scoped
    /// to a tenant.
    ///
    /// Returns `Some(seq)` with the sequence number assigned to the appended
    /// message, or `None` (a no-op) if the conversation does not exist or
    /// belongs to another tenant.
    async fn append_message(
        &self,
        tenant_id: Uuid,
        conversation_id: Uuid,
        message: &Message,
    ) -> Result<Option<i32>>;

    /// Deletes a conversation (and its messages) scoped to a tenant. Returns
    /// `true` if a conversation was deleted.
    async fn delete_conversation(&self, tenant_id: Uuid, id: Uuid) -> Result<bool>;
}
