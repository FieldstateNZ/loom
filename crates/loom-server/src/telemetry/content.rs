//! Privacy-gated content attachment: [`record_input_content`] and
//! [`record_output_content`].

use tracing::Span;

use super::env::capture_content;

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
