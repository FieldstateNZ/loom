//! `loom-provider-anthropic` — the Anthropic provider implementation.
//!
//! Translates Loom's fluent [`Conversation`] to Anthropic's native Messages API
//! and the response back, **losslessly** — server-side tool use, citations,
//! reasoning ("thinking") blocks, and anything Loom does not model natively are
//! preserved rather than flattened. The verbatim native response is kept on
//! [`Message::raw`] for audit and byte-equivalent replay.
//!
//! # What this crate provides
//!
//! - [`AnthropicProvider`] — an HTTP client for `POST /v1/messages` that
//!   implements the [`Provider`] trait: capability-negotiated
//!   [`complete`](Provider::complete) and SSE
//!   [`stream`](Provider::stream), and the pricing hook
//!   [`count_cost`](Provider::count_cost). It retries `429`/`5xx` with
//!   exponential backoff (honouring `retry-after`) and maps Anthropic error
//!   envelopes to [`ProviderError::Api`] with the native payload preserved.
//! - [`SseAccumulator`] — reassembles the streamed turn into the same domain
//!   [`Message`] the non-streaming path produces, and exposes the partial turn
//!   on mid-stream failure.
//! - [`translate`] — the pure request/response translation functions, exercised
//!   directly against fixtures.
//! - The Message Batches surface —
//!   [`create_batch`](AnthropicProvider::create_batch),
//!   [`get_batch`](AnthropicProvider::get_batch),
//!   [`fetch_batch_results`](AnthropicProvider::fetch_batch_results) and
//!   [`cancel_batch`](AnthropicProvider::cancel_batch) — for asynchronous bulk
//!   processing at the discounted batch tier, reusing the same client and auth.
//!
//! [`Message`]: loom_core::Message
//! - [`catalogue`] — the static catalogue of Claude models and their
//!   capabilities.
//!
//! [`Conversation`]: loom_core::Conversation
//! [`Message::raw`]: loom_core::Message::raw
//! [`Provider`]: loom_provider::Provider
//! [`ProviderError::Api`]: loom_provider::ProviderError::Api
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod batch;
mod catalogue;
mod provider;
mod streaming;
pub mod translate;

pub use batch::{AnthropicBatch, AnthropicBatchResult, BatchRequest, BatchRequestCounts};
pub use catalogue::{catalogue, feature_beta, BetaFeature, PROVIDER_NAME};
pub use provider::AnthropicProvider;
pub use streaming::SseAccumulator;

/// Re-export of the fluent conversation domain model.
pub use loom_core;
/// Re-export of the provider trait this crate implements.
pub use loom_provider;
