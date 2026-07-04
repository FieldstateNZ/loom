//! The typed content of a message.

use serde::{Deserialize, Serialize};

use super::{Citation, MediaSource};
use crate::CacheHint;

/// A single, typed piece of message content.
///
/// `ContentPart` is the heart of Loom's fluent conversation model. It is rich
/// enough to carry provider-native concepts — server-side tool use, citations,
/// reasoning ("thinking") blocks, and per-block prompt-cache markers — **without**
/// flattening them into a lossy OpenAI-shaped representation.
///
/// # Prompt caching
///
/// The cacheable variants ([`Text`](ContentPart::Text),
/// [`Image`](ContentPart::Image), [`Document`](ContentPart::Document),
/// [`ToolUse`](ContentPart::ToolUse), [`ToolResult`](ContentPart::ToolResult),
/// and [`Thinking`](ContentPart::Thinking)) each carry an optional
/// [`cache: Option<CacheHint>`](CacheHint) marking a cache breakpoint at that
/// block. The field is absent (rather than `null`) when unset, preserving
/// round-trip fidelity. Provider translators map it to the provider's native
/// marker (for Anthropic, `cache_control`).
///
/// # Serde representation
///
/// The enum is **internally tagged** with a `"type"` field and each variant is
/// rendered in `snake_case`. This makes the serialized form stable and
/// self-describing, e.g. a text part serializes as:
///
/// ```json
/// { "type": "text", "text": "hello" }
/// ```
///
/// The tag values are: `text`, `image`, `document`, `tool_use`, `tool_result`,
/// `server_tool_use`, `server_tool_result`, `thinking`, `redacted_thinking`,
/// and `provider_extension`.
///
/// # Extensibility
///
/// The enum is `#[non_exhaustive]`: new provider capabilities may add variants
/// in future releases. When a provider produces a shape Loom does not yet model
/// natively, translators should fall back to
/// [`ContentPart::ProviderExtension`] so that **no** provider feature is ever
/// silently dropped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentPart {
    /// A run of text, optionally annotated with source citations.
    Text {
        /// The literal text.
        text: String,
        /// Citations attributing spans of `text` to source documents.
        ///
        /// Absent (rather than an empty list) when the text carries no
        /// citations, preserving round-trip fidelity with providers that omit
        /// the field entirely.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        citations: Option<Vec<Citation>>,
        /// An optional prompt-cache breakpoint at this block.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<CacheHint>,
    },

    /// An image, supplied inline as base64 or by reference to a URL.
    Image {
        /// Where the image bytes come from.
        source: MediaSource,
        /// An optional prompt-cache breakpoint at this block.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<CacheHint>,
    },

    /// A document (e.g. a PDF), supplied inline as base64 or by reference.
    Document {
        /// Where the document bytes come from.
        source: MediaSource,
        /// An optional prompt-cache breakpoint at this block.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<CacheHint>,
    },

    /// A request from the assistant to invoke a **client-side** tool.
    ///
    /// The host application is expected to execute the named tool with `input`
    /// and return the result as a [`ContentPart::ToolResult`].
    ToolUse {
        /// Provider-assigned identifier correlating this call with its result.
        id: String,
        /// The tool's name.
        name: String,
        /// The tool's input arguments, as an opaque JSON value.
        input: serde_json::Value,
        /// An optional prompt-cache breakpoint at this block.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<CacheHint>,
    },

    /// The result of executing a **client-side** tool, sent back to the model.
    ToolResult {
        /// The [`ContentPart::ToolUse::id`] this result corresponds to.
        tool_use_id: String,
        /// The tool's output, as an opaque JSON value (a string, an array of
        /// content blocks, or any provider-specific shape).
        content: serde_json::Value,
        /// Whether the tool invocation failed. Absent when unspecified.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        /// An optional prompt-cache breakpoint at this block.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<CacheHint>,
    },

    /// A **provider-executed** tool invocation (e.g. Anthropic web search or
    /// code execution).
    ///
    /// Unlike [`ContentPart::ToolUse`], the host does not execute these — the
    /// provider runs them server-side and returns a
    /// [`ContentPart::ServerToolResult`] in the same response.
    ServerToolUse {
        /// Provider-assigned identifier correlating this call with its result.
        id: String,
        /// The server tool's name (e.g. `"web_search"`).
        name: String,
        /// The server tool's input arguments, as an opaque JSON value.
        input: serde_json::Value,
    },

    /// The result of a **provider-executed** tool.
    ///
    /// The native payload is preserved **verbatim** as a [`serde_json::Value`]
    /// so that replaying it back to the provider is byte-equivalent.
    ServerToolResult {
        /// The [`ContentPart::ServerToolUse::id`] this result corresponds to.
        tool_use_id: String,
        /// The provider's native result payload, preserved without
        /// interpretation.
        content: serde_json::Value,
    },

    /// A reasoning ("thinking") block emitted by the model.
    ///
    /// The `signature` must be preserved for cache and continuity correctness:
    /// providers reject conversations where thinking blocks have been modified
    /// or stripped of their signatures.
    Thinking {
        /// The reasoning text. May be empty when the provider omits the
        /// content but still returns a signed block.
        thinking: String,
        /// The opaque provider signature over the thinking block. Absent when
        /// the provider does not sign reasoning.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        /// An optional prompt-cache breakpoint at this block.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<CacheHint>,
    },

    /// A redacted reasoning block whose content the provider has withheld.
    ///
    /// The opaque `data` blob must be preserved and replayed verbatim to keep
    /// multi-turn reasoning continuity intact.
    RedactedThinking {
        /// The opaque, provider-encrypted reasoning payload.
        data: String,
    },

    /// An escape hatch for any provider feature Loom does not model natively.
    ///
    /// This guarantees losslessness: a translator that encounters an unknown
    /// content block can wrap it here rather than dropping it, and replay it
    /// back to the same provider unchanged.
    ProviderExtension {
        /// The provider that owns this payload (e.g. `"anthropic"`).
        provider: String,
        /// The provider-native block kind (e.g. `"mcp_tool_use"`).
        kind: String,
        /// The provider's native payload, preserved verbatim.
        payload: serde_json::Value,
    },
}

impl ContentPart {
    /// Constructs a plain [`ContentPart::Text`] part with no citations.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            citations: None,
            cache: None,
        }
    }
}
