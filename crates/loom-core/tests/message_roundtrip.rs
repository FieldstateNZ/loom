//! Serde round-trip tests for [`Message`].

mod common;

use common::assert_json_roundtrip;
use loom_core::{ContentPart, Message, Role, Usage};

#[test]
fn message_roundtrips() {
    let message = Message {
        role: Role::Assistant,
        content: vec![
            ContentPart::Thinking {
                thinking: "hmm".into(),
                signature: Some("sig".into()),
                cache: None,
            },
            ContentPart::text("done"),
        ],
        usage: Some({
            let mut u = Usage::new();
            u.output_tokens = Some(7);
            u
        }),
        // The verbatim native payload must survive its own serde round-trip.
        raw: Some(serde_json::json!({ "id": "msg_1", "type": "message" })),
    };
    assert_json_roundtrip(&message);

    for role in [Role::User, Role::Assistant, Role::Provider] {
        assert_json_roundtrip(&Message::new(role, vec![ContentPart::text("x")]));
    }
}
