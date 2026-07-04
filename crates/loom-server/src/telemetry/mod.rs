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
//! # Layout
//!
//! - [`env`] — env var names ([`env_keys`]), parsing helpers, and the
//!   content-capture toggle.
//! - [`guard`] — [`TelemetryGuard`], flushed and shut down on drop.
//! - [`init`] — [`init()`], installing the subscriber and (if configured) the
//!   OTLP exporters.
//! - [`metrics`] — [`Metrics`], the instrument bundle shared through
//!   [`AppState`](crate::state::AppState).
//! - [`request`] — [`trace_request`], the per-request span/metrics middleware.
//! - [`provider`] — [`provider_span`] and its usage/cost/stop-reason recorders.
//! - [`content`] — the privacy-gated prompt/completion content recorders.
//!
//! [`OTEL_SERVICE_NAME`]: https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/
//! [`OTEL_RESOURCE_ATTRIBUTES`]: https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/

mod content;
mod env;
mod guard;
mod init;
mod metrics;
mod provider;
mod request;

pub use content::{record_input_content, record_output_content};
pub use env::{capture_content, env_keys, set_capture_content};
pub use guard::TelemetryGuard;
pub use init::init;
pub use metrics::Metrics;
pub use provider::{provider_span, record_first_token, record_provider_result};
pub use request::{trace_request, RequestId, REQUEST_ID_HEADER};
