//! Per-provider-call span helpers: [`provider_span`] and its result recorders.

use std::time::Duration;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use tracing::field;
use tracing::Span;
use uuid::Uuid;

/// Builds the `provider.call` span nested under `parent`, pre-declaring every
/// usage/cost attribute as empty so they can be recorded once the turn resolves.
///
/// The span never carries message content by default; content is attached only
/// through [`record_content`](super::content), gated on
/// [`capture_content`](super::env::capture_content).
#[must_use]
pub fn provider_span(
    parent: &Span,
    provider: &str,
    model: &str,
    stream: bool,
    tenant: Uuid,
) -> Span {
    tracing::info_span!(
        parent: parent,
        "provider.call",
        otel.kind = "client",
        "gen_ai.system" = provider,
        "gen_ai.request.model" = model,
        "loom.stream" = stream,
        "loom.tenant.id" = %tenant,
        "gen_ai.usage.input_tokens" = field::Empty,
        "gen_ai.usage.output_tokens" = field::Empty,
        "gen_ai.usage.cache_read_tokens" = field::Empty,
        "gen_ai.usage.cache_write_tokens" = field::Empty,
        "loom.cost_usd" = field::Empty,
        "gen_ai.response.finish_reason" = field::Empty,
        "loom.first_token_ms" = field::Empty,
        "loom.input.content" = field::Empty,
        "loom.output.content" = field::Empty,
    )
}

/// Records a turn's usage, cost and stop reason as attributes on `span`.
///
/// Only counts, cost and the stop reason — never content. Unset usage fields are
/// left unrecorded so the exported span distinguishes "zero" from "not reported".
pub fn record_provider_result(
    span: &Span,
    usage: &loom_core::Usage,
    cost: Option<Decimal>,
    stop_reason: Option<&str>,
) {
    if let Some(v) = usage.input_tokens {
        span.record("gen_ai.usage.input_tokens", v);
    }
    if let Some(v) = usage.output_tokens {
        span.record("gen_ai.usage.output_tokens", v);
    }
    if let Some(v) = usage.cache_read_tokens {
        span.record("gen_ai.usage.cache_read_tokens", v);
    }
    if let Some(v) = usage.cache_write_tokens {
        span.record("gen_ai.usage.cache_write_tokens", v);
    }
    if let Some(cost) = cost.and_then(|c| c.to_f64()) {
        span.record("loom.cost_usd", cost);
    }
    if let Some(reason) = stop_reason {
        span.record("gen_ai.response.finish_reason", reason);
    }
}

/// Records the first-token latency (request start → first content delta) as an
/// attribute on `span` and emits it as a span event.
pub fn record_first_token(span: &Span, elapsed: Duration) {
    let ms = elapsed.as_secs_f64() * 1000.0;
    span.record("loom.first_token_ms", ms);
    span.in_scope(|| {
        tracing::info!(first_token_ms = ms, "first token received");
    });
}
