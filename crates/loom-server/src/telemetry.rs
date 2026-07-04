//! OpenTelemetry wiring: opt-in OTLP export of traces and metrics, layered on
//! top of the existing JSON structured logs.
//!
//! # Opt-in by endpoint
//!
//! Telemetry export is enabled solely by the presence of an OTLP endpoint
//! (`OTEL_EXPORTER_OTLP_ENDPOINT`, or the traces/metrics-specific variants).
//! When none is set the server still installs the JSON `tracing-subscriber`
//! logger but attaches no exporter, so tests and local development need no
//! collector. When an endpoint *is* set, an OTLP span exporter and metric
//! exporter are installed and a [`tracing_opentelemetry`] layer bridges the
//! existing `tracing` spans into OpenTelemetry.
//!
//! Both OTLP transports are supported and selected by the standard
//! `OTEL_EXPORTER_OTLP_PROTOCOL` variable: `grpc` (the default) uses tonic over
//! gRPC, while `http/protobuf` and `http/json` use the HTTP transport. Service
//! identity and resource attributes are read from the standard `OTEL_*`
//! variables ([`OTEL_SERVICE_NAME`], [`OTEL_RESOURCE_ATTRIBUTES`]).
//!
//! # Privacy
//!
//! By default **no prompt or completion content** is attached to any span,
//! metric, or log — only token counts, model, tenant, cost and stop reason. The
//! opt-in [`LOOM_TELEMETRY_CAPTURE_CONTENT`](env_keys::CAPTURE_CONTENT) flag
//! (set to `1`/`true`/`yes`/`on`) attaches message content to spans; it is a
//! debug/privacy-sensitive option and must never be enabled where telemetry
//! leaves a trusted boundary.
//!
//! [`OTEL_SERVICE_NAME`]: https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/
//! [`OTEL_RESOURCE_ATTRIBUTES`]: https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use http::header::HeaderValue;
use opentelemetry::metrics::{Counter, Histogram, UpDownCounter};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use tracing::field;
use tracing::Span;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::state::AppState;

/// The header carrying the per-request correlation id, read from the inbound
/// request (or generated) and echoed on the response.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Environment variable names read by the telemetry layer.
pub mod env_keys {
    /// Any OTLP endpoint variable whose presence turns export on.
    pub const OTLP_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
    /// Traces-specific OTLP endpoint override.
    pub const OTLP_TRACES_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT";
    /// Metrics-specific OTLP endpoint override.
    pub const OTLP_METRICS_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT";
    /// Transport selection: `grpc` (default), `http/protobuf`, or `http/json`.
    pub const OTLP_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";
    /// Logical service name (falls back to `loom-server`).
    pub const SERVICE_NAME: &str = "OTEL_SERVICE_NAME";
    /// Debug-only, privacy-sensitive: attach message content to spans when set.
    pub const CAPTURE_CONTENT: &str = "LOOM_TELEMETRY_CAPTURE_CONTENT";
}

/// The default logical service name when `OTEL_SERVICE_NAME` is unset.
const DEFAULT_SERVICE_NAME: &str = "loom-server";

/// Process-wide content-capture switch. Off by default; flipped on by
/// [`init`] when [`CAPTURE_CONTENT`](env_keys::CAPTURE_CONTENT) is truthy, and
/// settable directly by tests via [`set_capture_content`].
static CAPTURE_CONTENT: AtomicBool = AtomicBool::new(false);

/// Whether prompt/completion content may be attached to telemetry.
///
/// `false` by default. Every content-attaching call site must gate on this.
#[must_use]
pub fn capture_content() -> bool {
    CAPTURE_CONTENT.load(Ordering::Relaxed)
}

/// Overrides the content-capture switch. Intended for tests that assert the
/// privacy default and the opt-in behaviour without touching process env.
pub fn set_capture_content(enabled: bool) {
    CAPTURE_CONTENT.store(enabled, Ordering::Relaxed);
}

/// Parses a boolean-ish environment flag (`1`/`true`/`yes`/`on`).
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// A non-empty OTLP endpoint, if configured, preferring the signal-specific
/// override then the shared endpoint.
fn otlp_endpoint(specific: &str) -> Option<String> {
    std::env::var(specific)
        .ok()
        .or_else(|| std::env::var(env_keys::OTLP_ENDPOINT).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// The OTLP transport selected by `OTEL_EXPORTER_OTLP_PROTOCOL`, defaulting to
/// gRPC.
fn otlp_protocol() -> Protocol {
    match std::env::var(env_keys::OTLP_PROTOCOL)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "http/protobuf" | "http-protobuf" | "http" | "http/json" | "http-json" => {
            Protocol::HttpBinary
        }
        _ => Protocol::Grpc,
    }
}

/// The OpenTelemetry [`Resource`] describing this service, seeded from the
/// standard env vars with a `loom-server` fallback and the crate version.
fn resource() -> Resource {
    let service_name =
        std::env::var(env_keys::SERVICE_NAME).unwrap_or_else(|_| DEFAULT_SERVICE_NAME.to_owned());
    Resource::builder()
        .with_service_name(service_name)
        .with_attribute(KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_VERSION,
            loom_core::VERSION,
        ))
        .build()
}

/// A guard whose [`Drop`] flushes and shuts down the OTLP providers so buffered
/// spans and metrics are exported before the process exits. Holds nothing when
/// telemetry export is disabled.
#[must_use = "hold the guard for the process lifetime so telemetry is flushed on shutdown"]
pub struct TelemetryGuard {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
}

impl TelemetryGuard {
    /// A guard that owns no providers (export disabled).
    fn empty() -> Self {
        Self {
            tracer_provider: None,
            meter_provider: None,
        }
    }
}

/// Upper bound on how long process exit waits for the OTLP exporters to flush
/// and shut down before giving up. Generous enough for a healthy collector,
/// short enough that an unreachable one cannot wedge shutdown.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        let tracer_provider = self.tracer_provider.take();
        let meter_provider = self.meter_provider.take();
        if tracer_provider.is_none() && meter_provider.is_none() {
            // Export disabled: nothing to flush or shut down.
            return;
        }

        // `SdkTracerProvider::shutdown` / `SdkMeterProvider::shutdown` block the
        // calling thread while the batch (gRPC/tonic) exporter drains — and can
        // hang if the collector is unreachable. This guard is dropped at the end
        // of `main` *inside* the `#[tokio::main]` runtime, so calling `shutdown`
        // inline would block (or stall) a runtime worker. Run it on a dedicated
        // OS thread instead — the runtime stays free to drive the exporter's
        // background flush task — and wait on it only up to `SHUTDOWN_TIMEOUT`,
        // so a stuck collector cannot hang process exit.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("otel-shutdown".to_owned())
            .spawn(move || {
                if let Some(tp) = tracer_provider {
                    let _ = tp.shutdown();
                }
                if let Some(mp) = meter_provider {
                    let _ = mp.shutdown();
                }
                // Ignore send errors: the receiver may have already timed out.
                let _ = tx.send(());
            })
            .map_or_else(
                |_| {
                    // Thread spawn failed (extremely rare); nothing more we can
                    // safely do without risking a runtime-blocking inline call.
                },
                |_handle| {
                    // Bounded wait. On timeout we return and let the process exit;
                    // the detached thread is reclaimed by the OS. Any spans/metrics
                    // still buffered are dropped rather than blocking exit.
                    let _ = rx.recv_timeout(SHUTDOWN_TIMEOUT);
                },
            );
    }
}

/// Installs the tracing subscriber (JSON logs) and, when an OTLP endpoint is
/// configured, the OpenTelemetry trace and metric exporters.
///
/// Returns a [`TelemetryGuard`] that must be held for the lifetime of the
/// process; dropping it flushes the exporters. Must be called once, from within
/// a Tokio runtime (the OTLP exporters build async transports).
///
/// Never panics on an export-setup failure: it logs the error and falls back to
/// logs-only so a misconfigured collector cannot take the gateway down.
pub fn init() -> TelemetryGuard {
    set_capture_content(env_flag(env_keys::CAPTURE_CONTENT));

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer().json();

    let traces_endpoint = otlp_endpoint(env_keys::OTLP_TRACES_ENDPOINT);
    if traces_endpoint.is_none() {
        // Logs-only: no collector configured.
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .init();
        return TelemetryGuard::empty();
    }

    match build_exporters() {
        Ok((tracer_provider, meter_provider)) => {
            let tracer = tracer_provider.tracer("loom-server");
            global::set_tracer_provider(tracer_provider.clone());
            global::set_meter_provider(meter_provider.clone());
            let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt_layer)
                .with(otel_layer)
                .init();
            tracing::info!("OpenTelemetry OTLP export enabled");
            TelemetryGuard {
                tracer_provider: Some(tracer_provider),
                meter_provider: Some(meter_provider),
            }
        }
        Err(err) => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt_layer)
                .init();
            tracing::error!(error = %err, "failed to initialise OTLP exporters; continuing with logs only");
            TelemetryGuard::empty()
        }
    }
}

/// Builds the OTLP tracer and meter providers for the configured transport.
fn build_exporters() -> anyhow::Result<(SdkTracerProvider, SdkMeterProvider)> {
    let protocol = otlp_protocol();
    let resource = resource();

    // --- traces ---
    let span_exporter = match protocol {
        Protocol::Grpc => {
            let mut b = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_protocol(protocol);
            if let Some(ep) = otlp_endpoint(env_keys::OTLP_TRACES_ENDPOINT) {
                b = b.with_endpoint(ep);
            }
            b.build()?
        }
        _ => {
            let mut b = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_protocol(protocol);
            if let Some(ep) = otlp_endpoint(env_keys::OTLP_TRACES_ENDPOINT) {
                b = b.with_endpoint(ep);
            }
            b.build()?
        }
    };
    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(resource.clone())
        .build();

    // --- metrics ---
    let metric_exporter = match protocol {
        Protocol::Grpc => {
            let mut b = opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .with_protocol(protocol);
            if let Some(ep) = otlp_endpoint(env_keys::OTLP_METRICS_ENDPOINT) {
                b = b.with_endpoint(ep);
            }
            b.build()?
        }
        _ => {
            let mut b = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_protocol(protocol);
            if let Some(ep) = otlp_endpoint(env_keys::OTLP_METRICS_ENDPOINT) {
                b = b.with_endpoint(ep);
            }
            b.build()?
        }
    };
    let reader = PeriodicReader::builder(metric_exporter).build();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build();

    Ok((tracer_provider, meter_provider))
}

/// The metric instruments the gateway records, obtained from the global meter.
///
/// Constructed once and shared through [`AppState`]. When no meter provider is
/// installed (tests, local dev with no collector) the global meter is a no-op
/// and every `record_*` call is a cheap no-op.
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

/// A per-request correlation id, stored in the request extensions so handlers
/// and error responses can reference the same id carried by the request span.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// A low-cardinality route label derived from a request path.
///
/// Templates opaque segments (UUIDs and pure integers) to `{id}` so metric and
/// span cardinality stays bounded even without axum's `MatchedPath` (which is
/// not populated for a top-level middleware running before routing).
fn route_label(path: &str) -> String {
    if path == "/" {
        return "/".to_owned();
    }
    let mut out = String::with_capacity(path.len());
    for seg in path.split('/').skip(1) {
        out.push('/');
        if seg.is_empty() {
            continue;
        }
        if Uuid::parse_str(seg).is_ok() || seg.chars().all(|c| c.is_ascii_digit()) {
            out.push_str("{id}");
        } else {
            out.push_str(seg);
        }
    }
    out
}

/// Middleware that owns the HTTP request span, request-id propagation, and the
/// request-count/duration metrics.
///
/// It reads (or generates) the `x-request-id` header, attaches it to the request
/// span — so every log emitted while handling the request carries it — and
/// echoes it on the response. The span is the root of the per-request tree that
/// the turn, provider and store spans nest under.
pub async fn trace_request(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let route = route_label(req.uri().path());
    let request_id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let span = tracing::info_span!(
        "http.request",
        otel.name = %format!("{method} {route}"),
        otel.kind = "server",
        "http.request.method" = %method,
        "http.route" = %route,
        "loom.request_id" = %request_id,
        "http.response.status_code" = field::Empty,
    );

    req.extensions_mut().insert(RequestId(request_id.clone()));

    let start = Instant::now();
    let mut response = {
        use tracing::Instrument;
        next.run(req).instrument(span.clone()).await
    };
    let status = response.status();
    span.record("http.response.status_code", status.as_u16());

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    state
        .metrics()
        .record_http_request(&route, status.as_u16(), start.elapsed());

    response
}

/// Builds the `provider.call` span nested under `parent`, pre-declaring every
/// usage/cost attribute as empty so they can be recorded once the turn resolves.
///
/// The span never carries message content by default; content is attached only
/// through [`record_content`], gated on [`capture_content`].
#[must_use]
pub fn provider_span(
    parent: &Span,
    provider: &str,
    model: &str,
    stream: bool,
    tenant: Uuid,
) -> Span {
    tracing::info_span!(
        parent: parent,
        "provider.call",
        otel.kind = "client",
        "gen_ai.system" = provider,
        "gen_ai.request.model" = model,
        "loom.stream" = stream,
        "loom.tenant.id" = %tenant,
        "gen_ai.usage.input_tokens" = field::Empty,
        "gen_ai.usage.output_tokens" = field::Empty,
        "gen_ai.usage.cache_read_tokens" = field::Empty,
        "gen_ai.usage.cache_write_tokens" = field::Empty,
        "loom.cost_usd" = field::Empty,
        "gen_ai.response.finish_reason" = field::Empty,
        "loom.first_token_ms" = field::Empty,
        "loom.input.content" = field::Empty,
        "loom.output.content" = field::Empty,
    )
}

/// Records a turn's usage, cost and stop reason as attributes on `span`.
///
/// Only counts, cost and the stop reason — never content. Unset usage fields are
/// left unrecorded so the exported span distinguishes "zero" from "not reported".
pub fn record_provider_result(
    span: &Span,
    usage: &loom_core::Usage,
    cost: Option<Decimal>,
    stop_reason: Option<&str>,
) {
    if let Some(v) = usage.input_tokens {
        span.record("gen_ai.usage.input_tokens", v);
    }
    if let Some(v) = usage.output_tokens {
        span.record("gen_ai.usage.output_tokens", v);
    }
    if let Some(v) = usage.cache_read_tokens {
        span.record("gen_ai.usage.cache_read_tokens", v);
    }
    if let Some(v) = usage.cache_write_tokens {
        span.record("gen_ai.usage.cache_write_tokens", v);
    }
    if let Some(cost) = cost.and_then(|c| c.to_f64()) {
        span.record("loom.cost_usd", cost);
    }
    if let Some(reason) = stop_reason {
        span.record("gen_ai.response.finish_reason", reason);
    }
}

/// Records the first-token latency (request start → first content delta) as an
/// attribute on `span` and emits it as a span event.
pub fn record_first_token(span: &Span, elapsed: Duration) {
    let ms = elapsed.as_secs_f64() * 1000.0;
    span.record("loom.first_token_ms", ms);
    span.in_scope(|| {
        tracing::info!(first_token_ms = ms, "first token received");
    });
}

/// Attaches prompt (input) content to `span`, **only** when [`capture_content`]
/// is enabled. A no-op otherwise. The closure is not called unless capture is on,
/// so the (potentially large) digest is never built on the default path.
pub fn record_input_content(span: &Span, input: impl FnOnce() -> String) {
    if capture_content() {
        span.record("loom.input.content", input().as_str());
    }
}

/// Attaches completion (output) content to `span`, **only** when
/// [`capture_content`] is enabled. A no-op otherwise.
pub fn record_output_content(span: &Span, output: impl FnOnce() -> String) {
    if capture_content() {
        span.record("loom.output.content", output().as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::InMemoryMetricExporter;

    #[test]
    fn route_label_templates_ids() {
        assert_eq!(route_label("/healthz"), "/healthz");
        assert_eq!(route_label("/"), "/");
        assert_eq!(
            route_label("/v1/conversations/2f1a8c1e-0000-4000-8000-000000000000/turns"),
            "/v1/conversations/{id}/turns"
        );
        assert_eq!(route_label("/admin/keys/42"), "/admin/keys/{id}");
    }

    #[test]
    fn capture_content_defaults_off_and_toggles() {
        set_capture_content(false);
        assert!(!capture_content());
        set_capture_content(true);
        assert!(capture_content());
        set_capture_content(false);
    }

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
