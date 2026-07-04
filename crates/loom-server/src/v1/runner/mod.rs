//! The shared turn runner: [`execute_turn`] drives a resolved [`Provider`] for
//! both the stateful and stateless turn paths, records priced usage, persists
//! the assistant turn when applicable, and — for a streamed turn — reassembles
//! the SSE response via `stream::SseState` and settles it via
//! `stream::settle_stream_turn`.
//!
//! Split across three files by concern: this module holds `execute_turn`
//! itself plus the small digest/stop-reason helpers it shares with
//! [`stream`]; [`attribution`] holds usage attribution, pricing, and recording
//! (including [`attribution::turn_cost`], which wraps a priced amount as the
//! wire-level `TurnCost`); and [`stream`] holds the SSE streaming machinery.
//!
//! The non-streaming branch below returns [`TurnResponse`] — `{ message,
//! cost }` — and the streaming branch injects the same `cost` into the
//! terminal `turn_ended` frame (see `stream::SseState`), so both turn paths
//! carry Loom's authoritative, computed-once-at-turn-time price.

use std::time::Instant;

use axum::response::{IntoResponse, Response};
use axum::Json;
use http::header::HeaderValue;
use tracing::Instrument;
use uuid::Uuid;

use loom_core::{Conversation, ConversationOptions, Message};
use loom_provider::Provider;
use loom_store::ConversationStore;

use crate::budget::BudgetWarning;
use crate::error::ApiError;
use crate::state::AppState;
use crate::telemetry;

mod attribution;
mod stream;

use self::attribution::{record_turn_usage, turn_cost, UsageAttribution};
use self::stream::sse_response;
use super::turn_response::TurnResponse;

/// Response header set when a `warn`-action budget is over its soft limit.
const BUDGET_WARNING_HEADER: &str = "x-loom-budget-warning";

/// The shared core of both turn paths: runs the provider, records a priced
/// usage event for the turn, and — when `persist` is set — records the
/// assistant turn against `conversation`.
///
/// Non-streaming returns the assistant [`Message`] as JSON; streaming returns an
/// SSE response of [`TurnEvent`](loom_provider::TurnEvent) envelopes, recording
/// usage and persisting the reassembled assistant message when the stream ends.
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
        // `cost` is Loom's authoritative priced cost for this turn — the same
        // value just recorded to the usage outbox above, wrapped as the
        // wire-level `TurnCost` (never re-priced). See `turn_cost`'s doc for
        // the consistency semantics against `/v1/usage`.
        Json(TurnResponse {
            message,
            cost: turn_cost(cost),
        })
        .into_response()
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

/// The stable snake_case label for a [`StopReason`](loom_provider::StopReason),
/// mirroring its serde representation and preserving the verbatim string for
/// `Other`.
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
