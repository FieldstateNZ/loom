//! [`TelemetryGuard`]: flushes and shuts down the OTLP providers on drop.

use std::time::Duration;

use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

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
    pub(super) fn empty() -> Self {
        Self {
            tracer_provider: None,
            meter_provider: None,
        }
    }

    /// A guard owning the installed tracer and meter providers (export
    /// enabled).
    pub(super) fn new(
        tracer_provider: SdkTracerProvider,
        meter_provider: SdkMeterProvider,
    ) -> Self {
        Self {
            tracer_provider: Some(tracer_provider),
            meter_provider: Some(meter_provider),
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
