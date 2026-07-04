//! Turns a raw Anthropic SSE byte stream into a boxed [`TurnEvent`] stream,
//! folding each event through an [`SseAccumulator`].

use futures_util::stream::{self, BoxStream, StreamExt};
use loom_provider::{ProviderError, TurnEvent};
use serde_json::Value;

use super::accumulator::SseAccumulator;

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
