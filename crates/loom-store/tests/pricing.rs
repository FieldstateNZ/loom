//! Integration tests for the versioned pricing model, against a real
//! database.

mod common;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::json;

use loom_core::Usage;
use loom_store::{NewModelPrice, Pricer, PricingStore};

/// The seeded Anthropic prices load, and a newer price version supersedes the
/// old one only from its `effective_from` onward — older instants keep the old
/// price. This is the core of "price versioning affects new events only".
#[tokio::test]
async fn pricing_is_versioned_by_effective_from() {
    let (_pg, store) = common::setup().await;

    let early = Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap();
    let late = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();

    // The migration seeds opus at 5 / 25 effective 2026-01-01.
    let seeded = store
        .get_effective_price("anthropic", "claude-opus-4-8", early)
        .await
        .unwrap()
        .expect("seeded opus price");
    assert_eq!(seeded.input_per_mtok, Decimal::from(5));
    assert_eq!(seeded.output_per_mtok, Decimal::from(25));

    // A NEW version, effective 2026-07-01, never overwrites the old row.
    let bump_from = Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap();
    store
        .upsert_price(NewModelPrice {
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            input_per_mtok: Decimal::from(9),
            output_per_mtok: Decimal::from(45),
            cache_write_per_mtok: Decimal::new(1125, 2),
            cache_read_per_mtok: Decimal::new(90, 2),
            server_tool_prices: json!({ "web_search_requests": 0.02 }),
            batch_multiplier: rust_decimal::Decimal::ONE,
            currency: "USD".to_owned(),
            effective_from: bump_from,
        })
        .await
        .unwrap();

    // Before the bump: still the old price. After: the new one.
    let before = store
        .get_effective_price("anthropic", "claude-opus-4-8", early)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.input_per_mtok, Decimal::from(5));
    let after = store
        .get_effective_price("anthropic", "claude-opus-4-8", late)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.input_per_mtok, Decimal::from(9));

    // An unpriced model yields None (cost stays uncomputed, never an error).
    assert!(store
        .get_effective_price("anthropic", "no-such-model", late)
        .await
        .unwrap()
        .is_none());

    // The Pricer prices a cache-split usage against the seeded opus row.
    let mut usage = Usage::new();
    usage.input_tokens = Some(1_000_000);
    usage.output_tokens = Some(1_000_000);
    usage.cache_write_tokens = Some(1_000_000);
    usage.cache_read_tokens = Some(1_000_000);
    // 5 + 25 + 6.25 + 0.50 = 36.75
    assert_eq!(Pricer::cost(&usage, &seeded), Decimal::new(3675, 2));
}
