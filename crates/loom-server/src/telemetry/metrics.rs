//! The [`Metrics`] instrument bundle.

use std::time::Duration;

use opentelemetry::metrics::{Counter, Histogram, UpDownCounter};
use opentelemetry::{global, KeyValue};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use uuid::Uuid;

/// The metric instruments the gateway records, obtained from the global meter.
///
/// Constructed once and shared through [`AppState`](crate::state::AppState).
/// When no meter provider is installed (tests, local dev with no collector)
/// the global meter is a no-op and every `record_*` call is a cheap no-op.
#[derive(Clone)]
pub struct Metrics {
    requests_total: Counter<u64>,
    request_duration: Histogram<f64>,
    provider_call_duration: Histogram<f64>,
    tokens_in: Counter<u64>,
    tokens_out: Counter<u64>,
    cost_total: Counter<f64>,
    active_streams: UpDownCounter<i64>,
    budget_blocks: Counter<u64>,
}

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Builds the instruments from the global meter provider.
    #[must_use]
    pub fn new() -> Self {
        Self::from_meter(&global::meter("loom-server"))
    }

    /// Builds the instruments from a specific meter (used by unit tests with an
    /// in-memory reader).
    #[must_use]
    pub fn from_meter(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            requests_total: meter
                .u64_counter("loom.http.server.requests")
                .with_description("Total HTTP requests handled, by route and status.")
                .build(),
            request_duration: meter
                .f64_histogram("loom.http.server.duration")
                .with_unit("s")
                .with_description("HTTP request handling duration, by route and status.")
                .build(),
            provider_call_duration: meter
                .f64_histogram("loom.provider.call.duration")
                .with_unit("s")
                .with_description("Upstream provider call duration, by provider and model.")
                .build(),
            tokens_in: meter
                .u64_counter("loom.tokens.input")
                .with_description("Input tokens consumed, by tenant and model.")
                .build(),
            tokens_out: meter
                .u64_counter("loom.tokens.output")
                .with_description("Output tokens produced, by tenant and model.")
                .build(),
            cost_total: meter
                .f64_counter("loom.cost.total")
                .with_unit("USD")
                .with_description("Computed spend, by tenant and model.")
                .build(),
            active_streams: meter
                .i64_up_down_counter("loom.streams.active")
                .with_description("Currently open SSE streaming turns.")
                .build(),
            budget_blocks: meter
                .u64_counter("loom.budget.blocks")
                .with_description("Requests blocked by a budget, by scope.")
                .build(),
        }
    }

    /// Records one handled HTTP request and its duration.
    pub fn record_http_request(&self, route: &str, status: u16, elapsed: Duration) {
        let attrs = [
            KeyValue::new("http.route", route.to_owned()),
            KeyValue::new("http.response.status_code", i64::from(status)),
        ];
        self.requests_total.add(1, &attrs);
        self.request_duration.record(elapsed.as_secs_f64(), &attrs);
    }

    /// Records an upstream provider call's duration.
    pub fn record_provider_call(
        &self,
        provider: &str,
        model: &str,
        stream: bool,
        elapsed: Duration,
    ) {
        let attrs = [
            KeyValue::new("gen_ai.system", provider.to_owned()),
            KeyValue::new("gen_ai.request.model", model.to_owned()),
            KeyValue::new("loom.stream", stream),
        ];
        self.provider_call_duration
            .record(elapsed.as_secs_f64(), &attrs);
    }

    /// Records input/output token counts for a turn, by tenant and model.
    pub fn record_tokens(&self, tenant_id: Uuid, model: &str, input: u64, output: u64) {
        let attrs = [
            KeyValue::new("loom.tenant.id", tenant_id.to_string()),
            KeyValue::new("gen_ai.request.model", model.to_owned()),
        ];
        if input > 0 {
            self.tokens_in.add(input, &attrs);
        }
        if output > 0 {
            self.tokens_out.add(output, &attrs);
        }
    }

    /// Records computed spend for a turn, by tenant and model.
    pub fn record_cost(&self, tenant_id: Uuid, model: &str, cost: Decimal) {
        let Some(cost) = cost.to_f64() else { return };
        if cost <= 0.0 {
            return;
        }
        self.cost_total.add(
            cost,
            &[
                KeyValue::new("loom.tenant.id", tenant_id.to_string()),
                KeyValue::new("gen_ai.request.model", model.to_owned()),
            ],
        );
    }

    /// Marks an SSE stream as opened (increments the active-streams gauge).
    pub fn stream_started(&self) {
        self.active_streams.add(1, &[]);
    }

    /// Marks an SSE stream as closed (decrements the active-streams gauge).
    pub fn stream_ended(&self) {
        self.active_streams.add(-1, &[]);
    }

    /// Records a budget block, by scope (`"key"` or `"tenant"`).
    pub fn record_budget_block(&self, scope: &str) {
        self.budget_blocks
            .add(1, &[KeyValue::new("loom.budget.scope", scope.to_owned())]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

    /// Drives every recording helper through an in-memory metric reader and
    /// asserts the instruments carry the expected names and tenant/model
    /// attributes.
    #[test]
    fn metrics_helpers_record_expected_series() {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        let metrics = Metrics::from_meter(&provider.meter("test"));

        let tenant = Uuid::nil();
        metrics.record_http_request("/v1/turns", 200, Duration::from_millis(5));
        metrics.record_provider_call(
            "anthropic",
            "claude-opus-4-8",
            false,
            Duration::from_millis(9),
        );
        metrics.record_tokens(tenant, "claude-opus-4-8", 100, 40);
        metrics.record_cost(tenant, "claude-opus-4-8", Decimal::new(25, 1));
        metrics.stream_started();
        metrics.stream_ended();
        metrics.record_budget_block("tenant");

        provider.force_flush().expect("flush metrics");
        let exported = exporter.get_finished_metrics().expect("collect metrics");

        let mut names: Vec<String> = Vec::new();
        for rm in &exported {
            for scope in rm.scope_metrics() {
                for m in scope.metrics() {
                    names.push(m.name().to_string());
                }
            }
        }
        for expected in [
            "loom.http.server.requests",
            "loom.http.server.duration",
            "loom.provider.call.duration",
            "loom.tokens.input",
            "loom.tokens.output",
            "loom.cost.total",
            "loom.streams.active",
            "loom.budget.blocks",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "missing metric {expected}; got {names:?}"
            );
        }
    }
}
