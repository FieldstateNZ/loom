//! Anthropic's [Message Batches API] surface for asynchronous bulk processing.
//!
//! A batch is a set of independent Messages requests, each tagged with a
//! caller-chosen `custom_id`, submitted together and processed asynchronously at
//! Anthropic's discounted batch tier. These methods reuse the provider's
//! existing [`reqwest::Client`], auth headers, base-URL override and retry
//! policy (see
//! [`AnthropicProvider::send_json_request`](crate::AnthropicProvider::send_json_request)).
//!
//! The lifecycle is:
//!
//! 1. [`create_batch`](crate::AnthropicProvider::create_batch) — submit the
//!    requests;
//! 2. [`get_batch`](crate::AnthropicProvider::get_batch) — poll
//!    `processing_status` until it reaches `ended`;
//! 3. [`fetch_batch_results`](crate::AnthropicProvider::fetch_batch_results) —
//!    download the JSONL results document from the batch's `results_url`;
//! 4. [`cancel_batch`](crate::AnthropicProvider::cancel_batch) — request
//!    cancellation.
//!
//! The `params` of each request is a native Messages request body — exactly what
//! [`translate_request`](crate::translate::translate_request) produces — so
//! every Messages feature (tools, caching, thinking, …) is available in a batch
//! without a separate translation path.
//!
//! [Message Batches API]: https://docs.anthropic.com/en/api/creating-message-batches

mod counts;
mod provider_ext;
mod request;
mod result;
mod snapshot;

pub use counts::BatchRequestCounts;
pub use request::BatchRequest;
pub use result::AnthropicBatchResult;
pub use snapshot::AnthropicBatch;
