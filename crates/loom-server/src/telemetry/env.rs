//! Environment-driven configuration: the variable names ([`env_keys`]), their
//! parsing helpers, and the process-wide content-capture toggle.

use std::sync::atomic::{AtomicBool, Ordering};

use opentelemetry::KeyValue;
use opentelemetry_otlp::Protocol;
use opentelemetry_sdk::Resource;

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
/// [`init`](super::init::init) when
/// [`CAPTURE_CONTENT`](env_keys::CAPTURE_CONTENT) is truthy, and settable
/// directly by tests via [`set_capture_content`].
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
pub(super) fn env_flag(name: &str) -> bool {
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
pub(super) fn otlp_endpoint(specific: &str) -> Option<String> {
    std::env::var(specific)
        .ok()
        .or_else(|| std::env::var(env_keys::OTLP_ENDPOINT).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// The OTLP transport selected by `OTEL_EXPORTER_OTLP_PROTOCOL`, defaulting to
/// gRPC.
pub(super) fn otlp_protocol() -> Protocol {
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
pub(super) fn resource() -> Resource {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_content_defaults_off_and_toggles() {
        set_capture_content(false);
        assert!(!capture_content());
        set_capture_content(true);
        assert!(capture_content());
        set_capture_content(false);
    }
}
