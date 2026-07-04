//! Request-time options: sampling controls, tool definitions, and a
//! provider-specific options bag.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CacheHint, CacheNegotiation};

/// Options that shape how a provider should generate a response.
///
/// The common, cross-provider sampling controls are modelled as typed fields.
/// Anything provider-specific — Anthropic's `tool_choice` and `top_p`, cache
/// directives, and so on — lives in the [`provider_options`] bag, keyed by
/// provider name. This keeps the common path typed while never forcing a
/// provider feature to be expressed as a stringly-typed hack.
///
/// The type is `#[non_exhaustive]` and implements [`Default`], so callers can
/// build it up field by field:
///
/// ```
/// use loom_core::ConversationOptions;
///
/// let mut opts = ConversationOptions::new();
/// opts.temperature = Some(0.7);
/// opts.max_tokens = Some(1024);
/// assert_eq!(opts.temperature, Some(0.7));
/// ```
///
/// [`provider_options`]: ConversationOptions::provider_options
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConversationOptions {
    /// Sampling temperature. Provider-defined range; typically `0.0..=1.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// The maximum number of tokens the model may generate in its response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,

    /// Sequences that, when generated, cause the model to stop. Empty when
    /// unset.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,

    /// Definitions of the **client-side** tools the model may call. Empty when
    /// no tools are offered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,

    /// Opt-in automatic prompt caching for this request.
    ///
    /// When `true`, a provider translator deterministically places cache
    /// breakpoints for the (typically persisted) conversation — after the
    /// stable system-plus-tools head, and on the trailing history boundary —
    /// without the caller annotating individual blocks. This is the recommended
    /// path for persisted conversations, where per-block hints on reconstructed
    /// history would otherwise have to be re-applied every turn. Defaults to
    /// `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_cache: bool,

    /// How a provider should treat cache hints on a model that does not support
    /// prompt caching. Defaults to [`CacheNegotiation::SoftIgnore`] — cache
    /// hints are advisory.
    #[serde(default, skip_serializing_if = "is_default_negotiation")]
    pub cache_negotiation: CacheNegotiation,

    /// A per-provider bag of native options that Loom does not model as typed
    /// fields, keyed by provider name (e.g. `"anthropic"`).
    ///
    /// This is where provider-specific knobs live — `tool_choice`, `top_p`,
    /// `top_k`, thinking configuration, cache directives, and so on — as a
    /// JSON value the provider translator understands. A [`BTreeMap`] is used
    /// so serialization is deterministic. Empty when unset.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_options: BTreeMap<String, serde_json::Value>,
}

/// Whether a cache-negotiation policy is the default, so it can be omitted from
/// the serialized form.
fn is_default_negotiation(value: &CacheNegotiation) -> bool {
    *value == CacheNegotiation::default()
}

impl ConversationOptions {
    /// Constructs an empty set of options with every field at its default.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// The definition of a **client-side** tool the model may choose to call.
///
/// Server-side (provider-executed) tools are configured through the
/// [`provider_options`] bag rather than here, because their configuration is
/// provider-native.
///
/// [`provider_options`]: ConversationOptions::provider_options
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The tool's name, as the model will refer to it when calling.
    pub name: String,

    /// A natural-language description of what the tool does, used by the model
    /// to decide when to call it. Absent when the provider allows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// A JSON Schema describing the tool's input arguments.
    pub input_schema: serde_json::Value,

    /// An optional prompt-cache breakpoint on this tool definition.
    ///
    /// Tools render at the head of the request (before the system prompt and
    /// messages), so a breakpoint here caches the tool prefix. Absent (rather
    /// than `null`) when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<CacheHint>,
}
