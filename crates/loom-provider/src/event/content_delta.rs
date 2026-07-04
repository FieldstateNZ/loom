//! Incremental changes to a streaming content part, [`ContentDelta`].

use serde::{Deserialize, Serialize};

/// An incremental change to a streaming content part.
///
/// The enum is `#[non_exhaustive]`; match with a wildcard arm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentDelta {
    /// A chunk of text appended to a text part.
    Text {
        /// The appended text fragment.
        text: String,
    },
    /// A chunk of partial JSON appended to a tool-use input, streamed as it is
    /// generated (not yet parseable on its own).
    Json {
        /// The appended partial-JSON fragment.
        partial_json: String,
    },
    /// A chunk of reasoning text appended to a thinking part.
    Thinking {
        /// The appended reasoning fragment.
        thinking: String,
    },
    /// A chunk of the opaque signature over a thinking block (Anthropic's
    /// `signature_delta`). Accumulate these to reconstruct the final
    /// [`ContentPart::Thinking::signature`](loom_core::ContentPart::Thinking).
    SignatureDelta {
        /// The appended signature fragment.
        signature: String,
    },
    /// A citation appended to a text part (Anthropic's `citations_delta`),
    /// preserved verbatim.
    ///
    /// Accumulate these onto the text part's citations so a streamed cited text
    /// block reassembles identically to the non-streaming
    /// [`ContentPart::Text`](loom_core::ContentPart::Text) it corresponds to.
    Citation {
        /// The provider's native citation object, preserved without
        /// interpretation.
        citation: loom_core::Citation,
    },
}
