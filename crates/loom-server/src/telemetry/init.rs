//! Telemetry bootstrap: [`init`] installs the tracing subscriber and, when
//! configured, the OTLP trace and metric exporters.

use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use super::env::{self, env_keys};
use super::guard::TelemetryGuard;

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
    env::set_capture_content(env::env_flag(env_keys::CAPTURE_CONTENT));

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer().json();

    let traces_endpoint = env::otlp_endpoint(env_keys::OTLP_TRACES_ENDPOINT);
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
            TelemetryGuard::new(tracer_provider, meter_provider)
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
    let protocol = env::otlp_protocol();
    let resource = env::resource();

    // --- traces ---
    let span_exporter = match protocol {
        Protocol::Grpc => {
            let mut b = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_protocol(protocol);
            if let Some(ep) = env::otlp_endpoint(env_keys::OTLP_TRACES_ENDPOINT) {
                b = b.with_endpoint(ep);
            }
            b.build()?
        }
        _ => {
            let mut b = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_protocol(protocol);
            if let Some(ep) = env::otlp_endpoint(env_keys::OTLP_TRACES_ENDPOINT) {
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
            if let Some(ep) = env::otlp_endpoint(env_keys::OTLP_METRICS_ENDPOINT) {
                b = b.with_endpoint(ep);
            }
            b.build()?
        }
        _ => {
            let mut b = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_protocol(protocol);
            if let Some(ep) = env::otlp_endpoint(env_keys::OTLP_METRICS_ENDPOINT) {
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
