//! The shared turn runner: [`execute_turn`] drives a resolved [`Provider`] for
//! both the stateful and stateless turn paths, records priced usage, persists
//! the assistant turn when applicable, and — for a streamed turn — reassembles
//! the SSE response via [`SseState`] and settles it via [`settle_stream_turn`].

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use futures::{stream, StreamExt};
use http::header::HeaderValue;
use rust_decimal::Decimal;
use tracing::Instrument;
use uuid::Uuid;

use loom_core::{Conversation, ConversationOptions, Message, Role, Usage};
use loom_provider::{Provider, TurnEvent, TurnEventKind, TurnEventStream};
use loom_store::{ConversationStore, NewUsageEvent, Pricer, PricingStore};

use crate::budget::BudgetWarning;
use crate::error::ApiError;
use crate::state::AppState;
use crate::telemetry;

/// Response header set when a `warn`-action budget is over its soft limit.
const BUDGET_WARNING_HEADER: &str = "x-loom-budget-warning";

/// The identity a turn's usage is attributed to.
///
/// `conversation_id` is `Some` only for stateful turns; a stateless turn
/// records usage against the tenant and key with no conversation.
#[derive(Clone)]
struct UsageAttribution {
    tenant_id: Uuid,
    virtual_key_id: Option<Uuid>,
    conversation_id: Option<Uuid>,
    provider: String,
    model: String,
}

/// The shared core of both turn paths: runs the provider, records a priced
/// usage event for the turn, and — when `persist` is set — records the
/// assistant turn against `conversation`.
///
/// Non-streaming returns the assistant [`Message`] as JSON; streaming returns an
/// SSE response of [`TurnEvent`] envelopes, recording usage and persisting the
/// reassembled assistant message when the stream ends.
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_turn(
    provider: std::sync::Arc<dyn Provider>,
    state: &AppState,
    conversation: Conversation,
    options: ConversationOptions,
    stream: bool,
    persist: bool,
    key_id: Uuid,
    warning: Option<BudgetWarning>,
) -> Result<Response, ApiError> {
    let attribution = UsageAttribution {
        tenant_id: conversation.tenant_id,
        virtual_key_id: Some(key_id),
        // Stateless turns are not tied to a stored conversation.
        conversation_id: persist.then_some(conversation.id),
        provider: conversation.binding.provider.clone(),
        model: conversation.binding.model.clone(),
    };

    // The turn span roots the provider (and store) spans for this turn; it nests
    // under the HTTP request span the outer middleware opened. It carries the
    // tenant and model, never message content.
    let turn_span = tracing::info_span!(
        "conversation.turn",
        "loom.tenant.id" = %attribution.tenant_id,
        "gen_ai.system" = %attribution.provider,
        "gen_ai.request.model" = %attribution.model,
        "loom.stream" = stream,
    );
    let started = Instant::now();

    let mut response = if stream {
        let provider_span = telemetry::provider_span(
            &turn_span,
            &attribution.provider,
            &attribution.model,
            true,
            attribution.tenant_id,
        );
        let events = provider
            .stream(&conversation, &options)
            .instrument(provider_span.clone())
            .await
            .map_err(ApiError::from_provider)?;
        // Input content is attached only when the debug capture flag is set; the
        // span stays open across the whole stream so the output content and final
        // usage attributes are recorded when the stream settles.
        telemetry::record_input_content(&provider_span, || content_digest(&conversation));
        // The active-streams gauge is incremented/decremented by an RAII guard
        // owned by the SSE stream state (see `ActiveStreamGuard`), so it is
        // balanced on every termination path — including a mid-stream client
        // disconnect that drops the body future without a clean or error end.
        let persist_target = persist.then_some((conversation.tenant_id, conversation.id));
        sse_response(
            events,
            state.clone(),
            attribution,
            persist_target,
            provider_span,
            started,
        )
    } else {
        let provider_span = telemetry::provider_span(
            &turn_span,
            &attribution.provider,
            &attribution.model,
            false,
            attribution.tenant_id,
        );
        let message = provider
            .complete(&conversation, &options)
            .instrument(provider_span.clone())
            .await
            .map_err(ApiError::from_provider)?;
        state.metrics().record_provider_call(
            &attribution.provider,
            &attribution.model,
            false,
            started.elapsed(),
        );
        // Capture usage for every turn, before persisting the message.
        let usage = message.usage.clone().unwrap_or_default();
        let cost = record_turn_usage(state, &attribution, usage.clone()).await;
        telemetry::record_provider_result(
            &provider_span,
            &usage,
            cost,
            stop_reason_from_message(&message).as_deref(),
        );
        telemetry::record_input_content(&provider_span, || content_digest(&conversation));
        telemetry::record_output_content(&provider_span, || message_digest(&message));
        if persist {
            let store_span = tracing::info_span!(
                parent: &turn_span,
                "store.append_message",
                "loom.conversation.id" = %conversation.id,
            );
            state
                .store()
                .append_message(conversation.tenant_id, conversation.id, &message)
                .instrument(store_span)
                .await
                .map_err(ApiError::from_store)?;
        }
        Json(message).into_response()
    };

    // A `warn`-action budget over its soft limit lets the turn proceed but flags
    // it to the caller via a response header.
    if let Some(warning) = warning {
        if let Ok(value) = HeaderValue::from_str(warning.header_value()) {
            response.headers_mut().insert(BUDGET_WARNING_HEADER, value);
        }
    }
    Ok(response)
}

/// Records a priced usage event for a completed turn (best effort).
///
/// The cost is computed at write time from the effective price for
/// `(provider, model)` at the current instant; if no price is configured or the
/// lookup fails, the event is still recorded with `cost = None` so the raw usage
/// is never lost and cost can be recomputed later. The write itself goes through
/// the state's [`UsageRecorder`](crate::usage::UsageRecorder), which parks the
/// event in the outbox on failure — a usage-write fault never fails the turn.
async fn record_turn_usage(
    state: &AppState,
    attribution: &UsageAttribution,
    usage: Usage,
) -> Option<Decimal> {
    let cost = match state
        .store()
        .get_effective_price(&attribution.provider, &attribution.model, Utc::now())
        .await
    {
        Ok(Some(price)) => Some(Pricer::cost(&usage, &price)),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(error = %err, "price lookup failed; recording usage without cost");
            None
        }
    };
    // Debit the key's per-minute token bucket now that the turn's usage is known
    // (a no-op when the key has no token limit).
    if let Some(key_id) = attribution.virtual_key_id {
        let tokens = usage
            .input_tokens
            .unwrap_or(0)
            .saturating_add(usage.output_tokens.unwrap_or(0))
            .saturating_add(usage.cache_read_tokens.unwrap_or(0))
            .saturating_add(usage.cache_write_tokens.unwrap_or(0));
        state.rate_limiter().record_tokens(key_id, tokens);
    }
    // Emit token/cost metrics by tenant + model (no-ops without a meter).
    state.metrics().record_tokens(
        attribution.tenant_id,
        &attribution.model,
        usage.input_tokens.unwrap_or(0),
        usage.output_tokens.unwrap_or(0),
    );
    if let Some(cost) = cost {
        state
            .metrics()
            .record_cost(attribution.tenant_id, &attribution.model, cost);
    }
    let event = NewUsageEvent {
        tenant_id: attribution.tenant_id,
        virtual_key_id: attribution.virtual_key_id,
        conversation_id: attribution.conversation_id,
        provider: attribution.provider.clone(),
        model: attribution.model.clone(),
        usage,
        cost,
        is_batch: false,
    };
    state.usage_recorder().record(state.store(), event).await;
    cost
}

/// A best-effort stop-reason string for a completed non-streaming turn, read
/// from the provider's verbatim native response payload. Content is never read —
/// only the `stop_reason` field. `None` when absent.
fn stop_reason_from_message(message: &Message) -> Option<String> {
    message
        .raw
        .as_ref()?
        .get("stop_reason")?
        .as_str()
        .map(str::to_owned)
}

/// A compact, telemetry-only digest of a conversation's inputs (system prompt +
/// messages) as JSON. Built **only** when content capture is enabled.
fn content_digest(conversation: &Conversation) -> String {
    serde_json::json!({
        "system": conversation.system,
        "messages": conversation.messages,
    })
    .to_string()
}

/// A compact, telemetry-only digest of an assistant message as JSON. Built
/// **only** when content capture is enabled.
fn message_digest(message: &Message) -> String {
    serde_json::to_string(message).unwrap_or_default()
}

/// The stable snake_case label for a [`StopReason`], mirroring its serde
/// representation and preserving the verbatim string for `Other`.
fn stop_reason_label(reason: &loom_provider::StopReason) -> String {
    use loom_provider::StopReason;
    match reason {
        StopReason::EndTurn => "end_turn".to_owned(),
        StopReason::MaxTokens => "max_tokens".to_owned(),
        StopReason::StopSequence => "stop_sequence".to_owned(),
        StopReason::ToolUse => "tool_use".to_owned(),
        StopReason::PauseTurn => "pause_turn".to_owned(),
        StopReason::Refusal => "refusal".to_owned(),
        StopReason::Other(s) => s.clone(),
        _ => "unknown".to_owned(),
    }
}

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
fn sse_response(
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
        finalized: false,
        done: false,
        _gauge: gauge,
    };

    let body = stream::unfold(initial, |mut state| async move {
        if state.done {
            return None;
        }
        match state.events.next().await {
            Some(Ok(event)) => {
                state.accumulator.ingest(&event);
                // Record first-token latency (request start → first content
                // event) as a span event, once.
                if !state.first_token_seen && is_first_content(&event) {
                    telemetry::record_first_token(&state.span, state.started.elapsed());
                    state.first_token_seen = true;
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
    /// [`Provider`]-trait change), so it is deferred to a follow-up; the
    /// reassembled [`Message`] uses the provider-agnostic normalised events.
    async fn finalize(&mut self) {
        if self.finalized {
            return;
        }
        self.finalized = true;

        // Usage is finalised from the accumulator's message_delta/turn-end
        // snapshot and recorded for every streamed turn.
        settle_stream_turn(
            self.state.clone(),
            self.attribution.clone(),
            self.persist,
            self.span.clone(),
            self.started.elapsed(),
            self.accumulator.message(),
            self.accumulator.stop_reason_label(),
        )
        .await;
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
async fn settle_stream_turn(
    state: AppState,
    attribution: UsageAttribution,
    persist: Option<(Uuid, Uuid)>,
    span: tracing::Span,
    elapsed: Duration,
    message: Message,
    stop_reason: Option<String>,
) {
    let usage = message.usage.clone().unwrap_or_default();
    let cost = record_turn_usage(&state, &attribution, usage.clone()).await;

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
            TurnEventKind::TurnEnded { stop_reason, usage } => {
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

    /// The turn's stop reason as a telemetry label, if one was reported.
    fn stop_reason_label(&self) -> Option<String> {
        self.stop_reason.as_ref().map(stop_reason_label)
    }
}
