//! Anthropic Server-Sent Events (SSE) streaming: parsing the native event
//! stream, mapping it to Loom's [`TurnEvent`] envelope, and incrementally
//! reassembling the domain [`Message`].
//!
//! # Fidelity
//!
//! Every emitted [`TurnEvent`] carries the **verbatim** native event JSON in
//! [`TurnEvent::raw`]; the normalised [`TurnEventKind`] is only ever a
//! convenience view over it. No native event is dropped: keep-alives
//! (`ping`), the terminal `message_stop`, and any event type Loom does not
//! model are surfaced as [`TurnEventKind::Other`] carrying their native
//! `type`.
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
//! [`TurnEvent::raw`], so a consumer that folds the stream keeps a live,
//! partial [`Message`] at all times. If the stream fails mid-turn — a native
//! `error` event or a dropped connection — the consumer still holds every
//! event that arrived before the failure, and [`SseAccumulator::message`]
//! returns the turn assembled so far.

use std::collections::BTreeMap;

use futures_util::stream::{self, BoxStream, StreamExt};
use loom_core::Message;
use loom_provider::{ContentDelta, ProviderError, StopReason, TurnEvent, TurnEventKind};
use serde_json::{json, Map, Value};

use crate::translate;

/// Reassembles a native Anthropic Messages response from its SSE event stream.
///
/// Feed each native event (the JSON found on [`TurnEvent::raw`], or parsed
/// straight from an SSE `data:` line) to [`ingest`](Self::ingest). At any point
/// [`message`](Self::message) returns the domain [`Message`] assembled so far
/// and [`native_response`](Self::native_response) the reconstructed native
/// response object.
#[derive(Debug, Default)]
pub struct SseAccumulator {
    /// The message skeleton from `message_start` (id, role, model, usage, …),
    /// with `stop_reason`/`stop_sequence`/`usage` updated by `message_delta`.
    /// Content is rebuilt from `blocks` on demand rather than stored here.
    envelope: Value,
    /// Per-index content-block builders, keyed by the block's stream index.
    blocks: BTreeMap<usize, BlockBuilder>,
}

impl SseAccumulator {
    /// Creates an empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingests one native Anthropic SSE event, updating the assembled turn and
    /// returning the normalised [`TurnEvent`] to emit.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Api`] — carrying the verbatim event in
    /// `payload` — when the event is a native `error` event. The turn assembled
    /// so far is retained and remains readable via [`message`](Self::message).
    pub fn ingest(&mut self, event: Value) -> Result<TurnEvent, ProviderError> {
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();

        let kind = match event_type.as_str() {
            "message_start" => {
                if let Some(message) = event.get("message") {
                    self.envelope = message.clone();
                }
                TurnEventKind::TurnStarted
            }
            "content_block_start" => {
                let index = block_index(&event);
                let block = event.get("content_block").cloned().unwrap_or(Value::Null);
                self.blocks.insert(index, BlockBuilder::new(block.clone()));
                TurnEventKind::ContentPartStarted {
                    index,
                    part: translate::block_to_part(&block),
                }
            }
            "content_block_delta" => {
                let index = block_index(&event);
                let delta = event.get("delta").cloned().unwrap_or(Value::Null);
                self.apply_delta(index, &delta)
            }
            "content_block_stop" => {
                let index = block_index(&event);
                let block = self
                    .blocks
                    .get(&index)
                    .map_or_else(|| json!({}), BlockBuilder::finalized);
                TurnEventKind::ContentPartComplete {
                    index,
                    part: translate::block_to_part(&block),
                }
            }
            "message_delta" => {
                let delta = event.get("delta").cloned().unwrap_or(Value::Null);
                if let Value::Object(envelope) = &mut self.envelope {
                    if let Some(stop_reason) = delta.get("stop_reason") {
                        envelope.insert("stop_reason".into(), stop_reason.clone());
                    }
                    if let Some(stop_sequence) = delta.get("stop_sequence") {
                        envelope.insert("stop_sequence".into(), stop_sequence.clone());
                    }
                }
                if let Some(usage) = event.get("usage") {
                    self.merge_usage(usage);
                }
                let stop_reason = translate::stop_reason(&delta).unwrap_or(StopReason::EndTurn);
                let usage = self.envelope.get("usage").map(translate::translate_usage);
                TurnEventKind::TurnEnded { stop_reason, usage }
            }
            "ping" => TurnEventKind::Other {
                native_type: Some("ping".to_owned()),
            },
            "error" => return Err(error_from_event(&event)),
            // `message_stop` and any event Loom does not model are never
            // dropped: they surface with their native type, verbatim on `raw`.
            other => TurnEventKind::Other {
                native_type: Some(other.to_owned()),
            },
        };

        Ok(TurnEvent::new(kind, event))
    }

    /// The native Messages response reconstructed from the events seen so far,
    /// with content assembled from every (possibly still in-progress) block.
    #[must_use]
    pub fn native_response(&self) -> Value {
        let mut response = self.envelope.clone();
        let content: Vec<Value> = if let Some(&max) = self.blocks.keys().max() {
            (0..=max)
                .map(|index| {
                    self.blocks
                        .get(&index)
                        .map_or(Value::Null, BlockBuilder::finalized)
                })
                .collect()
        } else {
            Vec::new()
        };
        match &mut response {
            Value::Object(map) => {
                map.insert("content".into(), Value::Array(content));
            }
            _ => response = json!({ "content": content }),
        }
        response
    }

    /// The domain [`Message`] assembled so far.
    ///
    /// At stream end this equals what the non-streaming
    /// [`translate_response`](crate::translate::translate_response) would build
    /// for the same logical response. Mid-stream it is the partial turn.
    #[must_use]
    pub fn message(&self) -> Message {
        translate::translate_response(&self.native_response())
    }

    /// Applies a `content_block_delta` to the block at `index`, returning the
    /// normalised event kind.
    fn apply_delta(&mut self, index: usize, delta: &Value) -> TurnEventKind {
        let delta_type = delta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let builder = self.blocks.get_mut(&index);
        match delta_type {
            "text_delta" => {
                let text = string_field(delta, "text");
                if let Some(builder) = builder {
                    builder.text.push_str(&text);
                }
                TurnEventKind::ContentPartDelta {
                    index,
                    delta: ContentDelta::Text { text },
                }
            }
            "input_json_delta" => {
                let partial_json = string_field(delta, "partial_json");
                if let Some(builder) = builder {
                    builder.partial_json.push_str(&partial_json);
                }
                TurnEventKind::ContentPartDelta {
                    index,
                    delta: ContentDelta::Json { partial_json },
                }
            }
            "thinking_delta" => {
                let thinking = string_field(delta, "thinking");
                if let Some(builder) = builder {
                    builder.thinking.push_str(&thinking);
                }
                TurnEventKind::ContentPartDelta {
                    index,
                    delta: ContentDelta::Thinking { thinking },
                }
            }
            "signature_delta" => {
                let signature = string_field(delta, "signature");
                if let Some(builder) = builder {
                    builder.signature.push_str(&signature);
                }
                TurnEventKind::ContentPartDelta {
                    index,
                    delta: ContentDelta::SignatureDelta { signature },
                }
            }
            "citations_delta" => {
                let citation = delta.get("citation").cloned().unwrap_or(Value::Null);
                if let Some(builder) = builder {
                    builder.citations.push(citation.clone());
                }
                TurnEventKind::ContentPartDelta {
                    index,
                    delta: ContentDelta::Citation {
                        citation: loom_core::Citation(citation),
                    },
                }
            }
            // A `content_block_delta` subtype Loom does not model: the verbatim
            // delta rides on `raw`; nothing is dropped. Surface the delta's own
            // `type` (e.g. the subtype string) so consumers can distinguish it
            // from other unmodelled events.
            other => TurnEventKind::Other {
                native_type: Some(if other.is_empty() {
                    "content_block_delta".to_owned()
                } else {
                    other.to_owned()
                }),
            },
        }
    }

    /// Merges a native `usage` object into the envelope's usage, overlaying the
    /// incremental fields (`output_tokens`, `server_tool_use`, …) reported in
    /// `message_delta` over the initial `message_start` snapshot.
    fn merge_usage(&mut self, usage: &Value) {
        let Value::Object(delta) = usage else {
            return;
        };
        let Value::Object(envelope) = &mut self.envelope else {
            return;
        };
        let existing = envelope
            .entry("usage")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(existing) = existing {
            for (key, value) in delta {
                existing.insert(key.clone(), value.clone());
            }
        } else {
            *existing = Value::Object(delta.clone());
        }
    }
}

/// Accumulator for a single content block's streamed deltas.
#[derive(Debug, Default)]
struct BlockBuilder {
    /// The initial block from `content_block_start` (carries `type`, and any
    /// metadata such as a tool-use `id`/`name` or existing `citations`).
    initial: Value,
    /// Concatenated `text_delta` fragments.
    text: String,
    /// Concatenated `input_json_delta` fragments (parsed at finalisation).
    partial_json: String,
    /// Concatenated `thinking_delta` fragments.
    thinking: String,
    /// Concatenated `signature_delta` fragments.
    signature: String,
    /// Citations appended to a text block via `citations_delta`, preserved
    /// verbatim in arrival order.
    citations: Vec<Value>,
}

impl BlockBuilder {
    fn new(initial: Value) -> Self {
        Self {
            initial,
            ..Self::default()
        }
    }

    /// Produces the fully-assembled native block from the initial shape plus
    /// the accumulated deltas.
    fn finalized(&self) -> Value {
        let mut block = self.initial.clone();
        let Value::Object(map) = &mut block else {
            return block;
        };
        match map.get("type").and_then(Value::as_str).unwrap_or_default() {
            "text" => {
                map.insert("text".into(), json!(self.text));
                if !self.citations.is_empty() {
                    map.insert("citations".into(), Value::Array(self.citations.clone()));
                }
            }
            "thinking" => {
                map.insert("thinking".into(), json!(self.thinking));
                if !self.signature.is_empty() {
                    map.insert("signature".into(), json!(self.signature));
                }
            }
            kind if (kind == "tool_use"
                || kind == "server_tool_use"
                || kind.ends_with("_tool_use"))
                && !self.partial_json.is_empty() =>
            {
                let input = serde_json::from_str::<Value>(&self.partial_json)
                    .unwrap_or_else(|_| json!(self.partial_json));
                map.insert("input".into(), input);
            }
            _ => {}
        }
        block
    }
}

/// Reads the `index` field of a block event, defaulting to `0`.
fn block_index(event: &Value) -> usize {
    event
        .get("index")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .try_into()
        .unwrap_or(usize::MAX)
}

/// Reads a string field from a JSON object, defaulting to empty.
fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Maps a native `error` SSE event to [`ProviderError::Api`], preserving the
/// verbatim event in `payload`.
fn error_from_event(event: &Value) -> ProviderError {
    let message = event
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map_or_else(|| "anthropic stream error".to_owned(), str::to_owned);
    ProviderError::Api {
        status: None,
        message,
        payload: Some(event.clone()),
    }
}

/// Builds a boxed [`TurnEvent`] stream over an Anthropic SSE byte stream.
///
/// The returned stream is lazy and backpressure-aware: each SSE event is
/// parsed, folded into a fresh [`SseAccumulator`], and yielded before the next
/// is read — nothing is buffered ahead. A native `error` event yields a final
/// [`Err`] and ends the stream; a transport failure (dropped connection) yields
/// a final [`ProviderError::Transport`] and ends the stream.
pub(crate) fn event_stream(response: reqwest::Response) -> BoxStream<'static, StreamItem> {
    use eventsource_stream::Eventsource;

    let events = response.bytes_stream().eventsource();
    let state = StreamState {
        events: Box::pin(events),
        accumulator: SseAccumulator::new(),
    };

    stream::unfold(state, |mut state| async move {
        loop {
            match state.events.next().await {
                None => return None,
                Some(Ok(event)) => {
                    if event.data.is_empty() {
                        continue;
                    }
                    let value: Value = match serde_json::from_str(&event.data) {
                        Ok(value) => value,
                        Err(error) => {
                            return Some((
                                Err(ProviderError::Serialization(error.to_string())),
                                state.terminate(),
                            ));
                        }
                    };
                    let item = state.accumulator.ingest(value);
                    if item.is_err() {
                        return Some((item, state.terminate()));
                    }
                    return Some((item, state));
                }
                Some(Err(error)) => {
                    return Some((
                        Err(ProviderError::Transport(error.to_string())),
                        state.terminate(),
                    ));
                }
            }
        }
    })
    .boxed()
}

/// One item yielded by the Anthropic event stream.
type StreamItem = Result<TurnEvent, ProviderError>;

/// The `unfold` state driving [`event_stream`].
struct StreamState {
    events: BoxStream<'static, Result<eventsource_stream::Event, EventStreamError>>,
    accumulator: SseAccumulator,
}

impl StreamState {
    /// Terminates the stream after a final item by swapping in an exhausted
    /// event source, so the next `unfold` step reads `None` and ends.
    fn terminate(mut self) -> Self {
        self.events = stream::empty().boxed();
        self
    }
}

/// The error type produced by the SSE byte stream.
type EventStreamError = eventsource_stream::EventStreamError<reqwest::Error>;
