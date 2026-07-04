//! Anthropic Server-Sent Events (SSE) streaming: parsing the native event
//! stream, mapping it to Loom's [`TurnEvent`](loom_provider::TurnEvent)
//! envelope, and incrementally reassembling the domain
//! [`Message`](loom_core::Message).
//!
//! # Fidelity
//!
//! Every emitted [`TurnEvent`](loom_provider::TurnEvent) carries the
//! **verbatim** native event JSON in
//! [`TurnEvent::raw`](loom_provider::TurnEvent::raw); the normalised
//! [`TurnEventKind`](loom_provider::TurnEventKind) is only ever a convenience
//! view over it. No native event is dropped: keep-alives (`ping`), the
//! terminal `message_stop`, and any event type Loom does not model are
//! surfaced as
//! [`TurnEventKind::Other`](loom_provider::TurnEventKind::Other) carrying
//! their native `type`.
//!
//! # Accumulation and equivalence
//!
//! [`SseAccumulator`] reassembles the native Messages response from the event
//! stream — concatenating text, accumulating tool-input `partial_json` and
//! parsing it at block stop, and joining thinking text with its signature — so
//! that at stream end [`SseAccumulator::message`] equals **exactly** what the
//! non-streaming [`translate_response`](crate::translate::translate_response)
//! path would produce for the same logical response. The reassembly reuses the
//! same [`block_to_part`](crate::translate::block_to_part) mapping as the
//! non-streaming path, so a block built from deltas is indistinguishable from
//! one delivered whole.
//!
//! # Partial turns on failure
//!
//! The accumulator is public and driven from the native event on each
//! [`TurnEvent::raw`](loom_provider::TurnEvent::raw), so a consumer that
//! folds the stream keeps a live, partial [`Message`](loom_core::Message) at
//! all times. If the stream fails mid-turn — a native `error` event or a
//! dropped connection — the consumer still holds every event that arrived
//! before the failure, and [`SseAccumulator::message`] returns the turn
//! assembled so far.
//!
//! # Module layout
//!
//! [`accumulator`] holds [`SseAccumulator`], the reassembly state machine;
//! [`block_builder`] holds the per-content-block delta accumulator it drives;
//! [`event_stream`] holds the plumbing that turns a raw SSE byte stream into a
//! boxed [`TurnEvent`](loom_provider::TurnEvent) stream.

mod accumulator;
mod block_builder;
mod event_stream;

pub use accumulator::SseAccumulator;
pub(crate) use event_stream::event_stream;
