//! [`Pricer`]: computes a monetary [`Cost`](rust_decimal::Decimal) from a
//! [`Usage`] snapshot and an effective [`ModelPrice`].
//!
//! Pricing is pure arithmetic over the versioned price row — no database access
//! — so it is trivially unit-testable and the caller stays in control of *which*
//! price applies (it fetches the effective row via
//! [`PricingStore::get_effective_price`](crate::PricingStore::get_effective_price)
//! for the event's timestamp).

use std::str::FromStr;

use rust_decimal::Decimal;

use crate::model::ModelPrice;
use loom_core::Usage;

/// Computes costs from usage and a price.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pricer;

impl Pricer {
    /// Number of tokens a per-MTok rate is quoted against.
    const TOKENS_PER_MTOK: i64 = 1_000_000;

    /// Computes the total cost of `usage` under `price`.
    ///
    /// The cost is the sum of input, output, cache-read and cache-write token
    /// charges (each `tokens * per_mtok / 1_000_000`) plus per-request
    /// server-tool charges: for every entry in
    /// [`Usage::server_tool_use`], the invocation count is multiplied by the
    /// matching per-request price in
    /// [`ModelPrice::server_tool_prices`], if one is configured. A missing
    /// token figure counts as zero; a server tool with no configured price
    /// contributes nothing.
    #[must_use]
    pub fn cost(usage: &Usage, price: &ModelPrice) -> Decimal {
        let per_mtok = Decimal::from(Self::TOKENS_PER_MTOK);
        let mut total = Decimal::ZERO;

        let mut add_tokens = |tokens: Option<u64>, rate: Decimal| {
            let tokens = Decimal::from(tokens.unwrap_or(0));
            total += tokens * rate / per_mtok;
        };
        add_tokens(usage.input_tokens, price.input_per_mtok);
        add_tokens(usage.output_tokens, price.output_per_mtok);
        add_tokens(usage.cache_read_tokens, price.cache_read_per_mtok);
        add_tokens(usage.cache_write_tokens, price.cache_write_per_mtok);

        for (tool, count) in &usage.server_tool_use {
            if let Some(unit) = Self::server_tool_price(price, tool) {
                total += Decimal::from(*count) * unit;
            }
        }
        total
    }

    /// The per-request price for `tool` in `price.server_tool_prices`, if
    /// present and numeric.
    fn server_tool_price(price: &ModelPrice, tool: &str) -> Option<Decimal> {
        let value = price.server_tool_prices.get(tool)?;
        // Parse via the canonical string form so decimal literals such as
        // `0.01` round-trip exactly rather than through a lossy f64.
        Decimal::from_str(&value.to_string()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn price() -> ModelPrice {
        ModelPrice {
            id: Uuid::new_v4(),
            provider: "anthropic".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            input_per_mtok: Decimal::from(5),
            output_per_mtok: Decimal::from(25),
            cache_write_per_mtok: Decimal::from_str("6.25").unwrap(),
            cache_read_per_mtok: Decimal::from_str("0.50").unwrap(),
            server_tool_prices: serde_json::json!({ "web_search_requests": 0.01 }),
            currency: "USD".to_owned(),
            effective_from: Utc::now(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn prices_tokens_including_cache_split() {
        let mut usage = Usage::new();
        usage.input_tokens = Some(1_000_000);
        usage.output_tokens = Some(1_000_000);
        usage.cache_read_tokens = Some(1_000_000);
        usage.cache_write_tokens = Some(1_000_000);
        // 5 + 25 + 0.50 + 6.25 = 36.75
        assert_eq!(
            Pricer::cost(&usage, &price()),
            Decimal::from_str("36.75").unwrap()
        );
    }

    #[test]
    fn prices_server_tool_requests() {
        let mut usage = Usage::new();
        usage
            .server_tool_use
            .insert("web_search_requests".to_owned(), 3);
        // 3 * 0.01 = 0.03
        assert_eq!(
            Pricer::cost(&usage, &price()),
            Decimal::from_str("0.03").unwrap()
        );
    }

    #[test]
    fn unpriced_server_tool_is_free() {
        let mut usage = Usage::new();
        usage
            .server_tool_use
            .insert("code_execution_sessions".to_owned(), 9);
        assert_eq!(Pricer::cost(&usage, &price()), Decimal::ZERO);
    }
}
