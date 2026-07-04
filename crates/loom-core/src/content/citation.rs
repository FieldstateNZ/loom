//! Provider-native citation payloads.

use serde::{Deserialize, Serialize};

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
