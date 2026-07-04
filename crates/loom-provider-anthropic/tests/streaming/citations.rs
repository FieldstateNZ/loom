//! Citation streaming: `citations_delta` accumulation onto a text block, and
//! streamed/non-streamed message equivalence.

use loom_core::ContentPart;
use loom_provider::{ContentDelta, TurnEventKind};
use loom_provider_anthropic::translate;
use serde_json::{json, Value};

use super::support::{drive, kinds};

#[test]
fn citations_deltas_surface_as_citation_deltas_and_accumulate_onto_the_text_block() {
    let raw = include_str!("../fixtures/stream_citations.sse");
    let (results, _) = drive(raw);
    assert!(results.iter().all(Result::is_ok));
    let kinds = kinds(&results);

    // Each streamed citation surfaces as a normalised Citation delta carrying
    // the verbatim native citation object.
    assert!(kinds.contains(&TurnEventKind::ContentPartDelta {
        index: 0,
        delta: ContentDelta::Citation {
            citation: loom_core::Citation(json!({
                "type": "char_location",
                "cited_text": "grass is green",
                "document_index": 0,
                "document_title": "Colours",
                "start_char_index": 10,
                "end_char_index": 24
            })),
        },
    }));

    // The completed text block joins its text and carries both citations, in
    // order, matching the non-streaming ContentPart::Text { citations } shape.
    assert!(kinds.contains(&TurnEventKind::ContentPartComplete {
        index: 0,
        part: ContentPart::Text {
            text: "The grass is green, the sky is blue".to_owned(),
            citations: Some(vec![
                loom_core::Citation(json!({
                    "type": "char_location",
                    "cited_text": "grass is green",
                    "document_index": 0,
                    "document_title": "Colours",
                    "start_char_index": 10,
                    "end_char_index": 24
                })),
                loom_core::Citation(json!({
                    "type": "char_location",
                    "cited_text": "sky is blue",
                    "document_index": 0,
                    "document_title": "Colours",
                    "start_char_index": 40,
                    "end_char_index": 51
                })),
            ]),
            cache: None,
        },
    }));
}

#[test]
fn streamed_cited_text_message_equals_the_non_streaming_message() {
    let raw = include_str!("../fixtures/stream_citations.sse");
    let (_, accumulator) = drive(raw);

    let non_streaming: Value =
        serde_json::from_str(include_str!("../fixtures/stream_citations_response.json"))
            .expect("valid fixture");
    let expected = translate::translate_response(&non_streaming);

    // The streamed citations are not dropped: the reassembled cited text block
    // is byte-for-byte the non-streaming Message, citations intact.
    assert_eq!(
        accumulator.message(),
        expected,
        "streamed cited message must equal the non-streaming message"
    );
    assert!(matches!(
        expected.content.first(),
        Some(ContentPart::Text { citations: Some(citations), .. }) if citations.len() == 2
    ));
}
