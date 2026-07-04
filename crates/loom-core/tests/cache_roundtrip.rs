//! Serde round-trip tests for [`CacheHint`] and [`CacheNegotiation`].

mod common;

use common::assert_json_roundtrip;
use loom_core::{CacheHint, CacheNegotiation, CacheTtl, ConversationOptions};
use serde_json::json;

#[test]
fn cache_hints_roundtrip_and_default_omits_fields() {
    assert_json_roundtrip(&CacheHint::ephemeral());
    assert_json_roundtrip(&CacheHint::with_ttl(CacheTtl::FiveMinutes));
    assert_json_roundtrip(&CacheHint::with_ttl(CacheTtl::OneHour));
    assert_json_roundtrip(&CacheNegotiation::SoftIgnore);
    assert_json_roundtrip(&CacheNegotiation::HardFail);

    // A default (no-TTL) hint serializes as an empty object; TTLs use
    // provider-agnostic names.
    assert_eq!(
        serde_json::to_value(CacheHint::ephemeral()).unwrap(),
        json!({})
    );
    assert_eq!(
        serde_json::to_value(CacheHint::with_ttl(CacheTtl::OneHour)).unwrap(),
        json!({ "ttl": "one_hour" })
    );

    // Default options omit the caching knobs entirely (stable prefix bytes).
    let value = serde_json::to_value(ConversationOptions::new()).unwrap();
    assert!(value.get("auto_cache").is_none());
    assert!(value.get("cache_negotiation").is_none());
}
