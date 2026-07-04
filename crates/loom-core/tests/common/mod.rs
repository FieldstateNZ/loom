//! Shared test helpers for `loom-core`'s integration test suite.

/// Asserts that a value survives a JSON serialize -> deserialize cycle
/// unchanged.
pub fn assert_json_roundtrip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let encoded = serde_json::to_string(value).expect("serialize");
    let decoded: T = serde_json::from_str(&encoded).expect("deserialize");
    assert_eq!(value, &decoded, "round-trip mismatch for {encoded}");
}
