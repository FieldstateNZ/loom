//! Partial-turn recovery: a mid-stream native `error` event and an early
//! disconnect must both leave the turn assembled so far readable.

use loom_core::{ContentPart, Role};
use loom_provider::ProviderError;
use serde_json::json;

use super::support::drive;

#[test]
fn mid_stream_error_is_surfaced_and_the_partial_turn_is_preserved() {
    let raw = include_str!("../fixtures/stream_error.sse");
    let (results, accumulator) = drive(raw);

    // The final event is an error, surfaced as a structured Api error whose
    // payload is the verbatim native error event.
    let last = results.last().expect("at least one result");
    match last {
        Err(ProviderError::Api {
            status,
            message,
            payload,
        }) => {
            assert_eq!(*status, None);
            assert_eq!(message, "Overloaded");
            assert_eq!(
                payload.as_ref().unwrap()["error"]["type"],
                json!("overloaded_error")
            );
        }
        other => panic!("expected Api error, got {other:?}"),
    }

    // Everything before the error is still available as a partial turn.
    let partial = accumulator.message();
    assert_eq!(partial.role, Role::Assistant);
    assert_eq!(partial.content.len(), 1);
    assert_eq!(partial.content[0], ContentPart::text("Partial answer"));
}

#[test]
fn early_disconnect_leaves_the_partial_turn_readable() {
    // The transcript ends abruptly mid-block, with no content_block_stop,
    // message_delta, or message_stop — as a dropped connection would.
    let raw = include_str!("../fixtures/stream_disconnect.sse");
    let (results, accumulator) = drive(raw);
    assert!(results.iter().all(Result::is_ok));

    // The in-progress text block is assembled from the deltas seen so far.
    let partial = accumulator.message();
    assert_eq!(partial.content.len(), 1);
    assert_eq!(partial.content[0], ContentPart::text("Interrupted mid"));
}
