//! Plain-text streaming: the expected event sequence, verbatim `raw`
//! preservation, and streamed/non-streamed message equivalence.

use loom_core::ContentPart;
use loom_provider::{ContentDelta, StopReason, TurnEventKind};
use loom_provider_anthropic::translate;
use serde_json::Value;

use super::support::{drive, kinds, native_events};

#[test]
fn plain_text_maps_to_the_expected_event_sequence() {
    let raw = include_str!("../fixtures/stream_text.sse");
    let (results, _) = drive(raw);
    assert!(results.iter().all(Result::is_ok));

    let kinds = kinds(&results);
    assert_eq!(kinds[0], TurnEventKind::TurnStarted);
    assert_eq!(
        kinds[1],
        TurnEventKind::ContentPartStarted {
            index: 0,
            part: ContentPart::text(""),
        }
    );
    assert_eq!(
        kinds[2],
        TurnEventKind::ContentPartDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Hello".to_owned()
            },
        }
    );
    assert_eq!(
        kinds[3],
        TurnEventKind::ContentPartDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: ", world".to_owned()
            },
        }
    );
    assert_eq!(
        kinds[4],
        TurnEventKind::ContentPartComplete {
            index: 0,
            part: ContentPart::text("Hello, world"),
        }
    );
    match &kinds[5] {
        TurnEventKind::TurnEnded {
            stop_reason,
            usage,
            cost,
        } => {
            assert_eq!(*stop_reason, StopReason::EndTurn);
            let usage = usage.as_ref().expect("message_delta couples usage");
            assert_eq!(usage.input_tokens, Some(10));
            assert_eq!(usage.output_tokens, Some(7));
            // The provider layer never prices a turn — only loom-server does,
            // injecting `cost` into the outgoing frame from its pricing table.
            assert!(cost.is_none());
        }
        other => panic!("expected TurnEnded, got {other:?}"),
    }
    assert_eq!(
        kinds[6],
        TurnEventKind::Other {
            native_type: Some("message_stop".to_owned()),
        }
    );
}

#[test]
fn every_event_carries_the_verbatim_native_json_on_raw() {
    let raw = include_str!("../fixtures/stream_text.sse");
    let events = native_events(raw);
    let (results, _) = drive(raw);
    assert_eq!(results.len(), events.len());
    for (result, native) in results.iter().zip(&events) {
        let event = result.as_ref().expect("event ok");
        assert_eq!(&event.raw, native, "raw must be the verbatim native event");
    }
}

#[test]
fn streamed_text_message_equals_the_non_streaming_message() {
    let raw = include_str!("../fixtures/stream_text.sse");
    let (_, accumulator) = drive(raw);

    let non_streaming: Value =
        serde_json::from_str(include_str!("../fixtures/stream_text_response.json"))
            .expect("valid fixture");
    let expected = translate::translate_response(&non_streaming);

    assert_eq!(
        accumulator.message(),
        expected,
        "streamed-accumulated message must equal the non-streaming message"
    );
}
