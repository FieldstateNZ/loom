//! A provider pass is fully validated before its tools run. Native metadata in
//! completed parts survives replay; only the final answer is exposed as text.
use crate::error::ApiError;
use futures::StreamExt;
use loom_core::{Message, Role, Usage};
use loom_provider::{StopReason, TurnEventKind, TurnEventStream};
use std::collections::BTreeMap;

pub(super) struct Pass {
    pub(super) result: Result<(Message, StopReason), ApiError>,
    pub(super) usage: Option<Usage>,
}
pub(super) async fn collect(mut events: TurnEventStream) -> Pass {
    let mut parts = BTreeMap::new();
    let mut usage = None;
    let mut stop = None;
    let mut count = 0;
    let mut bytes: usize = 0;
    let mut failed = false;
    while let Some(event) = events.next().await {
        count += 1;
        if count > 16384 {
            failed = true;
            break;
        }
        let Ok(event) = event else {
            failed = true;
            break;
        };
        match event.kind {
            TurnEventKind::ContentPartComplete { index, part } => {
                bytes =
                    bytes.saturating_add(serde_json::to_vec(&part).map_or(usize::MAX, |v| v.len()));
                if stop.is_some()
                    || bytes > 4 * 1024 * 1024
                    || parts.len() > 1024
                    || parts.insert(index, part).is_some()
                {
                    failed = true;
                    break;
                }
            }
            TurnEventKind::Usage(u) => usage = Some(u),
            TurnEventKind::TurnEnded {
                stop_reason,
                usage: final_usage,
                ..
            } => {
                if stop.is_some() {
                    failed = true;
                    break;
                }
                stop = Some(stop_reason);
                if final_usage.is_some() {
                    usage = final_usage;
                }
            }
            _ => {}
        }
    }
    let result = if let Some(stop) = stop.filter(|_| !failed && !parts.is_empty()) {
        Ok((
            Message {
                role: Role::Assistant,
                content: parts.into_values().collect(),
                usage: usage.clone(),
                raw: None,
            },
            stop,
        ))
    } else {
        Err(ApiError::unavailable(
            "private memory model response was interrupted or incomplete; start a new conversation",
        ))
    };
    Pass { result, usage }
}
