//! Content parts — the provider-faithful building blocks of a [`Message`].
//!
//! [`Message`]: crate::Message

use serde::{Deserialize, Serialize};

/// A single, typed piece of message content.
///
/// `ContentPart` is the heart of Loom's fluent conversation model. It is rich
/// enough to carry provider-native concepts — server-side tool use, cache
/// markers, citations, and reasoning ("thinking") blocks — **without**
/// flattening them into a lossy OpenAI-shaped representation.
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
    },

    /// An image, supplied inline as base64 or by reference to a URL.
    Image {
        /// Where the image bytes come from.
        source: MediaSource,
    },

    /// A document (e.g. a PDF), supplied inline as base64 or by reference.
    Document {
        /// Where the document bytes come from.
        source: MediaSource,
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
        }
    }
}

/// The origin of an image or document's bytes.
///
/// Serialized as an internally tagged enum with a `"type"` field
/// (`base64` or `url`), mirroring the shape used by providers such as
/// Anthropic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaSource {
    /// Bytes supplied inline, base64-encoded.
    Base64 {
        /// The IANA media type of the data (e.g. `"image/png"`).
        media_type: String,
        /// The base64-encoded bytes.
        data: String,
    },
    /// Bytes referenced by URL rather than inlined.
    Url {
        /// The URL the provider should fetch the media from.
        url: String,
    },
}

/// A citation attributing a span of generated text to a source.
///
/// Provider citation shapes vary widely (character ranges, page ranges, web
/// search result locations, …). To remain lossless and provider-faithful,
/// `Citation` wraps the provider's native citation object verbatim rather than
/// flattening it into a single normalized form. It serializes transparently as
/// that inner value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Citation(
    /// The provider's native citation payload, preserved without
    /// interpretation.
    pub serde_json::Value,
);
