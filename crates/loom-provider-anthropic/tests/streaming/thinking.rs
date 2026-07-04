//! Thinking-block streaming: joined text + signature deltas, the never-dropped
//! `ping` keep-alive, and streamed/non-streamed message equivalence.

use loom_core::ContentPart;
use loom_provider::{ContentDelta, TurnEventKind};
use loom_provider_anthropic::translate;
use serde_json::Value;

use super::support::{drive, kinds};

#[test]
fn thinking_deltas_and_signature_assemble_and_ping_is_never_dropped() {
    let raw = include_str!("../fixtures/stream_thinking.sse");
    let (results, _) = drive(raw);
    let kinds = kinds(&results);

    // The ping keep-alive surfaces as Other rather than being dropped.
    assert!(kinds.contains(&TurnEventKind::Other {
        native_type: Some("ping".to_owned()),
    }));

    // The thinking deltas and the signature delta are distinct delta kinds.
    assert!(kinds.contains(&TurnEventKind::ContentPartDelta {
        index: 0,
        delta: ContentDelta::Thinking {
            thinking: "The user asked ".to_owned()
        },
    }));
    assert!(kinds.contains(&TurnEventKind::ContentPartDelta {
        index: 0,
        delta: ContentDelta::SignatureDelta {
            signature: "EqoBsig==".to_owned()
        },
    }));

    // The completed thinking block joins its text and carries the signature.
    assert!(kinds.contains(&TurnEventKind::ContentPartComplete {
        index: 0,
        part: ContentPart::Thinking {
            thinking: "The user asked a simple question.".to_owned(),
            signature: Some("EqoBsig==".to_owned()),
            cache: None,
        },
    }));
}

#[test]
fn streamed_thinking_message_equals_the_non_streaming_message() {
    let raw = include_str!("../fixtures/stream_thinking.sse");
    let (_, accumulator) = drive(raw);

    let non_streaming: Value =
        serde_json::from_str(include_str!("../fixtures/stream_thinking_response.json"))
            .expect("valid fixture");
    let expected = translate::translate_response(&non_streaming);

    assert_eq!(
        accumulator.message(),
        expected,
        "streamed thinking message must equal the non-streaming message"
    );
}
