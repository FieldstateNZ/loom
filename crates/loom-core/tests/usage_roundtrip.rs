//! Serde round-trip tests for [`Usage`].

mod common;

use common::assert_json_roundtrip;
use loom_core::Usage;
use serde_json::json;

#[test]
fn usage_roundtrips() {
    let mut usage = Usage::new();
    usage.input_tokens = Some(100);
    usage.output_tokens = Some(42);
    usage.cache_read_tokens = Some(10);
    usage.cache_write_tokens = Some(5);
    usage
        .server_tool_use
        .insert("web_search_requests".to_owned(), 3);
    usage.raw = Some(json!({ "input_tokens": 100, "service_tier": "standard" }));
    assert_json_roundtrip(&usage);
    assert_json_roundtrip(&Usage::new());
}
