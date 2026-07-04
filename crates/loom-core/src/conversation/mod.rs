//! Conversations and their binding to a provider.

#[allow(clippy::module_inception)]
mod conversation;
mod provider_binding;

pub use conversation::Conversation;
pub use provider_binding::ProviderBinding;
