//! Shared SSE-transcript driving helpers for the streaming test modules.

use loom_provider::{ProviderError, TurnEvent, TurnEventKind};
use loom_provider_anthropic::SseAccumulator;
use serde_json::Value;

/// Splits a raw SSE transcript into its ordered native event JSON payloads.
///
/// Events are separated by a blank line; the `data:` line(s) of each event
/// carry the JSON. Non-`data` fields (`event:`) are ignored — the JSON `type`
/// is authoritative, exactly as the streaming accumulator treats it.
pub(crate) fn native_events(raw: &str) -> Vec<Value> {
    raw.split("\n\n")
        .filter_map(|block| {
            let data: String = block
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|rest| rest.strip_prefix(' ').unwrap_or(rest))
                .collect::<Vec<_>>()
                .join("\n");
            if data.trim().is_empty() {
                return None;
            }
            Some(serde_json::from_str(&data).expect("valid event JSON"))
        })
        .collect()
}

/// Drives an accumulator over every event of a transcript, returning the
/// per-event results and the accumulator (so the assembled/partial message can
/// be inspected).
pub(crate) fn drive(raw: &str) -> (Vec<Result<TurnEvent, ProviderError>>, SseAccumulator) {
    let mut accumulator = SseAccumulator::new();
    let results = native_events(raw)
        .into_iter()
        .map(|event| accumulator.ingest(event))
        .collect();
    (results, accumulator)
}

pub(crate) fn kinds(results: &[Result<TurnEvent, ProviderError>]) -> Vec<TurnEventKind> {
    results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .map(|event| event.kind.clone())
        .collect()
}
