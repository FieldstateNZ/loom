//! Where an image or document's bytes come from.

use serde::{Deserialize, Serialize};

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
