//! Client `tool_use` streaming: `partial_json` accumulation and
//! streamed/non-streamed message equivalence.

use loom_core::ContentPart;
use loom_provider::{ContentDelta, TurnEventKind};
use loom_provider_anthropic::translate;
use serde_json::{json, Value};

use super::support::{drive, kinds};

#[test]
fn tool_use_input_json_deltas_assemble_into_a_parsed_tool_call() {
    let raw = include_str!("../fixtures/stream_tool_use.sse");
    let (results, _) = drive(raw);
    let kinds = kinds(&results);

    // The tool_use block is announced with an empty input object.
    assert_eq!(
        kinds[4],
        TurnEventKind::ContentPartStarted {
            index: 1,
            part: ContentPart::ToolUse {
                id: "toolu_01".to_owned(),
                name: "get_weather".to_owned(),
                input: json!({}),
                cache: None,
            },
        }
    );
    // Input arrives as partial-JSON fragments.
    assert_eq!(
        kinds[5],
        TurnEventKind::ContentPartDelta {
            index: 1,
            delta: ContentDelta::Json {
                partial_json: "{\"location\":".to_owned()
            },
        }
    );
    // At block stop the fragments are parsed into the completed call.
    assert_eq!(
        kinds[7],
        TurnEventKind::ContentPartComplete {
            index: 1,
            part: ContentPart::ToolUse {
                id: "toolu_01".to_owned(),
                name: "get_weather".to_owned(),
                input: json!({ "location": "Wellington, NZ" }),
                cache: None,
            },
        }
    );
}

#[test]
fn streamed_tool_use_message_equals_the_non_streaming_message() {
    let raw = include_str!("../fixtures/stream_tool_use.sse");
    let (_, accumulator) = drive(raw);

    let non_streaming: Value =
        serde_json::from_str(include_str!("../fixtures/stream_tool_use_response.json"))
            .expect("valid fixture");
    let expected = translate::translate_response(&non_streaming);

    assert_eq!(accumulator.message(), expected);
}
