//! `loom-core` — Loom's **fluent conversation** domain model.
//!
//! This crate owns the provider-agnostic abstraction that Loom exposes to
//! clients: conversations, messages, content parts, usage and options.
//! Provider libraries translate this model to and from each provider's native
//! wire format **without flattening provider-specific capabilities** — Loom
//! carries provider-native concepts (server-side tool use, citations, reasoning
//! blocks) faithfully rather than forcing them through a lossy OpenAI-shaped
//! normalisation.
//!
//! # Design guarantees
//!
//! - **Lossless round-trip.** Any provider response can be represented such
//!   that replaying it back to the same provider is semantically
//!   byte-equivalent. Anything not modelled natively is preserved through
//!   [`ContentPart::ProviderExtension`] or a raw [`serde_json::Value`] field.
//! - **No OpenAI-shape assumptions.** The model is built around one fluent
//!   conversation abstraction, not a chat-completions envelope.
//! - **Stable, self-describing serialisation.** [`ContentPart`] is internally
//!   tagged with a `"type"` field; see its documentation for the tag values.
//!
//! # Example
//!
//! Constructing a small conversation:
//!
//! ```
//! use loom_core::{Conversation, Message, ProviderBinding, Role};
//! use uuid::Uuid;
//!
//! let tenant = Uuid::new_v4();
//! let mut conversation =
//!     Conversation::new(tenant, ProviderBinding::new("anthropic", "claude-opus-4-8"));
//! conversation.system = Some("You are a helpful assistant.".to_owned());
//! conversation.messages.push(Message::user("Hello, Loom!"));
//! conversation
//!     .messages
//!     .push(Message::assistant("Hello — how can I help?"));
//!
//! assert_eq!(conversation.binding.provider, "anthropic");
//! assert_eq!(conversation.messages.len(), 2);
//! assert_eq!(conversation.messages[0].role, Role::User);
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod cache;
mod content;
mod conversation;
mod message;
mod options;
mod usage;

pub use cache::{CacheHint, CacheNegotiation, CacheTtl};
pub use content::{Citation, ContentPart, MediaSource};
pub use conversation::{Conversation, ProviderBinding};
pub use message::{Message, Role};
pub use options::{ConversationOptions, McpServerRef, ServerTool, ToolDefinition};
pub use usage::Usage;

/// The crate version, sourced from Cargo at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
