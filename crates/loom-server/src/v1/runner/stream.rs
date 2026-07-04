//! SSE streaming machinery for a streamed turn.
//!
//! [`sse_response`] builds the SSE [`Response`] and drives the provider's
//! event stream via [`SseState`]; [`TurnAccumulator`] reassembles the
//! assistant [`Message`] from the provider's normalised events; and
//! [`settle_stream_turn`] records usage and persists the reassembled message
//! when the stream ends — whether cleanly, on a provider error, or (via
//! [`SseState`]'s [`Drop`]) on a mid-stream client disconnect.
//!
//! Pricing is a special case of "when the stream ends": the terminal
//! `turn_ended` frame must carry its own `cost` (see [`TurnEventKind::TurnEnded`]),
//! so [`SseState::price_and_record`] runs the pricing-and-recording step
//! *inline*, the moment a `TurnEnded` event is about to be forwarded to the
//! client, rather than waiting for the underlying stream to actually end. The
//! result is cached on [`SseState::recorded_cost`] so the later
//! [`settle_stream_turn`] — triggered by the true end of stream, a provider
//! error, or [`Drop`] — never prices or records the same turn's usage twice.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use futures::{stream, StreamExt};
use http::header::HeaderValue;
use rust_decimal::Decimal;
use tracing::Instrument;
use uuid::Uuid;

use loom_core::{Message, Role, Usage};
use loom_provider::{TurnCost, TurnEvent, TurnEventKind, TurnEventStream};
use loom_store::ConversationStore;

use crate::error::ApiError;
use crate::state::AppState;
use crate::telemetry;

use super::attribution::{record_turn_usage, turn_cost, UsageAttribution};
use super::{message_digest, stop_reason_label};

/// An RAII guard that keeps the `loom.streams.active` gauge balanced.
///
/// Constructing one increments the gauge; dropping one decrements it. Because
/// the decrement lives in [`Drop`] it fires on *every* stream termination path —
/// a clean end, a provider error, and (critically) a mid-stream client
/// disconnect, where hyper drops the SSE body future without the unfold ever
/// reaching its terminal branch. The gauge decrement is intentionally the guard's
/// *sole* responsibility (it is not also done in [`SseState::finalize`]) so a
/// stream can never be double-counted.
struct ActiveStreamGuard {
    metrics: telemetry::Metrics,
}

impl ActiveStreamGuard {
    /// Marks a stream as open (increments the active-streams gauge). The paired
    /// decrement fires when the guard is dropped.
    fn new(metrics: telemetry::Metrics) -> Self {
        metrics.stream_started();
        Self { metrics }
    }
}

impl Drop for ActiveStreamGuard {
    fn drop(&mut self) {
        // Synchronous and safe in `Drop`: a bare gauge decrement, no `.await`.
        self.metrics.stream_ended();
    }
}

/// The mutable state driving the SSE `unfold`.
struct SseState {
    events: TurnEventStream,
    accumulator: TurnAccumulator,
    state: AppState,
    attribution: UsageAttribution,
    /// `(tenant_id, conversation_id)` for message persistence, or `None` for a
    /// stateless turn. Usage is recorded regardless.
    persist: Option<(Uuid, Uuid)>,
    /// The provider span, kept open across the whole stream so first-token
    /// latency, final usage and the stop reason are recorded on it.
    span: tracing::Span,
    /// When the turn started, for first-token latency and provider duration.
    started: Instant,
    /// Whether the first-token event has already been recorded.
    first_token_seen: bool,
    /// The turn's priced cost, cached the moment it is first computed —
    /// `None` until then, `Some(cost)` once [`SseState::price_and_record`] has
    /// run (with `cost` itself possibly `None`, when unpriced). Guards
    /// [`settle_stream_turn`] against pricing and recording the same turn's
    /// usage twice: once inline, when the terminal `turn_ended` event is
    /// forwarded (the common case), and again — only if that never
    /// happened — when the stream settles.
    recorded_cost: Option<Option<Decimal>>,
    finalized: bool,
    done: bool,
    /// Balances the active-streams gauge for the lifetime of the stream. Held
    /// only for its [`Drop`] side effect (decrement on every termination path);
    /// never read directly.
    _gauge: ActiveStreamGuard,
}

/// Builds the SSE [`Response`] for a streamed turn.
///
/// Each provider [`TurnEvent`] is serialised verbatim (normalised envelope plus
/// raw native event) as one `data:` frame. When the underlying stream ends —
/// cleanly or via a provider error — a priced usage event is recorded and, for
/// a stateful turn, the reassembled assistant [`Message`] is persisted (best
/// effort). If the client instead disconnects mid-stream (the SSE body future is
/// dropped before either terminal branch runs), [`SseState`]'s own [`Drop`]
/// settles the partial turn off-thread on a best-effort basis, and the
/// active-streams gauge is balanced by [`ActiveStreamGuard`] regardless.
pub(super) fn sse_response(
    events: TurnEventStream,
    app_state: AppState,
    attribution: UsageAttribution,
    persist: Option<(Uuid, Uuid)>,
    span: tracing::Span,
    started: Instant,
) -> Response {
    // Increment the active-streams gauge now; the guard's `Drop` decrements it on
    // whichever way the stream ends.
    let gauge = ActiveStreamGuard::new(app_state.metrics().clone());
    let initial = SseState {
        events,
        accumulator: TurnAccumulator::new(),
        state: app_state,
        attribution,
        persist,
        span,
        started,
        first_token_seen: false,
        recorded_cost: None,
        finalized: false,
        done: false,
        _gauge: gauge,
    };

    let body = stream::unfold(initial, |mut state| async move {
        if state.done {
            return None;
        }
        match state.events.next().await {
            Some(Ok(mut event)) => {
                state.accumulator.ingest(&event);
                // Record first-token latency (request start → first content
                // event) as a span event, once.
                if !state.first_token_seen && is_first_content(&event) {
                    telemetry::record_first_token(&state.span, state.started.elapsed());
                    state.first_token_seen = true;
                }
                // The terminal event: price and record the turn's usage now —
                // inline, exactly once (see `price_and_record`) — so its
                // authoritative cost can ride on this very frame instead of
                // making the client wait for `/v1/usage`'s asynchronous rollup.
                if let TurnEventKind::TurnEnded { cost, .. } = &mut event.kind {
                    *cost = state.price_and_record().await;
                }
                let frame = Event::default()
                    .json_data(&event)
                    .unwrap_or_else(|_| Event::default().data("{}"));
                Some((Ok::<Event, Infallible>(frame), state))
            }
            Some(Err(err)) => {
                // Settle whatever was assembled before the failure, then emit a
                // terminal error frame and end the stream.
                state.finalize().await;
                let envelope = ApiError::from_provider(err).envelope();
                let frame = Event::default()
                    .event("error")
                    .json_data(&envelope)
                    .unwrap_or_else(|_| Event::default().event("error").data("{}"));
                state.done = true;
                Some((Ok(frame), state))
            }
            None => {
                state.finalize().await;
                None
            }
        }
    });

    let mut response = Sse::new(body).into_response();
    // Discourage proxy buffering so events flush to the client promptly.
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

impl SseState {
    /// Settles the turn once the stream ends: records a priced usage event
    /// (always) and, for a stateful turn, persists the reassembled assistant
    /// message. Idempotent — a clean end, an error end and the [`Drop`]-path
    /// best-effort settlement cannot both run it, because the first to run sets
    /// the `finalized` flag the others check.
    ///
    /// Best effort throughout: a usage-write or persistence failure is logged,
    /// never surfaced mid-stream.
    ///
    /// # Known limitation: `Message.raw` asymmetry between the turn paths
    ///
    /// The message persisted here has `raw = None`, whereas the non-streaming
    /// path persists `raw = Some(native_payload)` (the provider's verbatim
    /// response blob). This is a deliberate, documented gap — not data loss on
    /// the wire: every SSE frame already carries the verbatim native event in
    /// its [`TurnEvent::raw`](loom_provider::TurnEvent) field, so the streaming
    /// client sees the full native payload. What is missing is only the *single
    /// reconstructed native-response blob* at persistence time. Rebuilding it
    /// would require provider-specific accumulation of native events (a
    /// [`Provider`](loom_provider::Provider)-trait change), so it is deferred to
    /// a follow-up; the reassembled [`Message`] uses the provider-agnostic
    /// normalised events.
    async fn finalize(&mut self) {
        if self.finalized {
            return;
        }
        self.finalized = true;

        // Usage is finalised from the accumulator's message_delta/turn-end
        // snapshot and recorded for every streamed turn. `recorded_cost` is
        // `Some` already when the terminal `turn_ended` event was seen (the
        // common case — see `price_and_record`), so this is a no-op re-price;
        // it is `None` only when the stream ended without ever reporting
        // `TurnEnded` (a provider error or a disconnect before turn end), in
        // which case `settle_stream_turn` prices it here for the first time.
        settle_stream_turn(
            self.state.clone(),
            self.attribution.clone(),
            self.persist,
            self.span.clone(),
            self.started.elapsed(),
            self.accumulator.message(),
            self.accumulator.stop_reason_label(),
            self.recorded_cost,
        )
        .await;
    }

    /// Prices and records this turn's usage exactly once, from the
    /// accumulator's usage snapshot so far, caching the priced [`Decimal`] in
    /// [`recorded_cost`](SseState::recorded_cost) so a later
    /// [`finalize`](Self::finalize) or [`Drop`] settlement — triggered when the
    /// underlying stream actually ends — reuses the cached value instead of
    /// pricing and recording the turn's usage a second time.
    ///
    /// Called when the terminal `TurnEnded` event is observed, so its
    /// authoritative cost can be embedded in that same outgoing frame; returns
    /// the wire-level [`TurnCost`] to embed there.
    async fn price_and_record(&mut self) -> Option<TurnCost> {
        if let Some(cost) = self.recorded_cost {
            return turn_cost(cost);
        }
        let usage = self.accumulator.usage();
        let cost = record_turn_usage(&self.state, &self.attribution, usage).await;
        self.recorded_cost = Some(cost);
        turn_cost(cost)
    }
}

impl Drop for SseState {
    /// Best-effort settlement for a stream dropped before [`finalize`] ran — a
    /// mid-stream client disconnect, where hyper drops the SSE body future
    /// without the unfold reaching its clean-end (`None`) or error branch.
    ///
    /// The `finalized` flag guards against double-recording: if [`finalize`]
    /// already ran, this is a no-op (the active-streams gauge is still balanced
    /// by the `_gauge` field's own `Drop`, which runs after this). Otherwise the
    /// partial turn — the work the provider *did* do before the client went away
    /// — is recorded and persisted on a detached task, since `Drop` cannot
    /// `.await`. Handles are cloned out of `self`; the accumulated [`Usage`] is
    /// whatever the stream reported before the disconnect (often empty, as the
    /// usage snapshot rides the terminal event).
    ///
    /// [`finalize`]: SseState::finalize
    fn drop(&mut self) {
        if self.finalized {
            return;
        }
        // Spawning needs a Tokio runtime; if the state is dropped outside one
        // (e.g. during runtime teardown) skip the best-effort persist rather than
        // panic. The gauge is still balanced by `_gauge`.
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        self.finalized = true;
        handle.spawn(settle_stream_turn(
            self.state.clone(),
            self.attribution.clone(),
            self.persist,
            self.span.clone(),
            self.started.elapsed(),
            self.accumulator.message(),
            self.accumulator.stop_reason_label(),
            self.recorded_cost,
        ));
    }
}

/// Records the priced usage event and provider-duration metric for a streamed
/// turn and, for a stateful turn, persists the reassembled assistant message.
///
/// Shared by [`SseState::finalize`] (the clean/error end) and [`SseState`]'s
/// [`Drop`] (a mid-stream client disconnect) so the two paths record the turn
/// identically. Best effort throughout: a usage-write or persistence failure is
/// logged, never surfaced. Does **not** touch the active-streams gauge — that is
/// owned solely by [`ActiveStreamGuard`].
///
/// `recorded_cost` is [`SseState::recorded_cost`] at settlement time: `Some`
/// when [`SseState::price_and_record`] already priced and recorded this turn's
/// usage (the terminal `turn_ended` event was seen and forwarded), in which
/// case that cached value is reused as-is; `None` when it never ran — a
/// provider error or a client disconnect before `turn_ended` — in which case
/// this call prices and records the usage for the first (and only) time.
/// Either way the turn's usage is priced and recorded exactly once.
#[allow(clippy::too_many_arguments)]
async fn settle_stream_turn(
    state: AppState,
    attribution: UsageAttribution,
    persist: Option<(Uuid, Uuid)>,
    span: tracing::Span,
    elapsed: Duration,
    message: Message,
    stop_reason: Option<String>,
    recorded_cost: Option<Option<Decimal>>,
) {
    let usage = message.usage.clone().unwrap_or_default();
    let cost = match recorded_cost {
        Some(cost) => cost,
        None => record_turn_usage(&state, &attribution, usage.clone()).await,
    };

    // Settle the span kept open across the stream: final usage, cost, stop
    // reason and (only when opted in) the output content.
    telemetry::record_provider_result(&span, &usage, cost, stop_reason.as_deref());
    telemetry::record_output_content(&span, || message_digest(&message));
    state
        .metrics()
        .record_provider_call(&attribution.provider, &attribution.model, true, elapsed);

    if let Some((tenant_id, conversation_id)) = persist {
        let store_span = tracing::info_span!(
            parent: &span,
            "store.append_message",
            "loom.conversation.id" = %conversation_id,
        );
        if let Err(err) = state
            .store()
            .append_message(tenant_id, conversation_id, &message)
            .instrument(store_span)
            .await
        {
            tracing::error!(
                error = %err,
                conversation_id = %conversation_id,
                "failed to persist streamed assistant turn"
            );
        }
    }
}

/// Whether an event carries the first assistant content — the trigger for the
/// first-token latency measurement.
fn is_first_content(event: &TurnEvent) -> bool {
    matches!(
        event.kind,
        TurnEventKind::ContentPartStarted { .. }
            | TurnEventKind::ContentPartDelta { .. }
            | TurnEventKind::ContentPartComplete { .. }
    )
}

/// Reassembles the assistant [`Message`] from a provider's normalised
/// [`TurnEvent`] stream.
///
/// This works over the provider-agnostic [`TurnEventKind`] envelope — completed
/// content parts and the final usage snapshot — so it reassembles any provider's
/// turn identically, without reaching into a provider's native wire format.
#[derive(Default)]
struct TurnAccumulator {
    parts: BTreeMap<usize, loom_core::ContentPart>,
    usage: Option<Usage>,
    stop_reason: Option<loom_provider::StopReason>,
}

impl TurnAccumulator {
    fn new() -> Self {
        Self::default()
    }

    /// Folds one event into the assembled turn.
    fn ingest(&mut self, event: &TurnEvent) {
        match &event.kind {
            TurnEventKind::ContentPartComplete { index, part } => {
                self.parts.insert(*index, part.clone());
            }
            TurnEventKind::Usage(usage) => {
                self.usage = Some(usage.clone());
            }
            // `cost` is never populated by a provider (only loom-server injects
            // it, into the outgoing frame, after this fold already ran — see
            // `SseState::price_and_record`), so it is never folded here.
            TurnEventKind::TurnEnded {
                stop_reason, usage, ..
            } => {
                self.stop_reason = Some(stop_reason.clone());
                if let Some(usage) = usage {
                    self.usage = Some(usage.clone());
                }
            }
            _ => {}
        }
    }

    /// The assistant [`Message`] assembled from the events seen so far, with
    /// content parts in ascending index order.
    fn message(&self) -> Message {
        let content = self.parts.values().cloned().collect();
        let mut message = Message::new(Role::Assistant, content);
        message.usage = self.usage.clone();
        message
    }

    /// The [`Usage`] accumulated so far — an empty snapshot if none has been
    /// reported yet. Used to price the turn as soon as its usage is known,
    /// without waiting to reassemble the full [`Message`].
    fn usage(&self) -> Usage {
        self.usage.clone().unwrap_or_default()
    }

    /// The turn's stop reason as a telemetry label, if one was reported.
    fn stop_reason_label(&self) -> Option<String> {
        self.stop_reason.as_ref().map(stop_reason_label)
    }
}
