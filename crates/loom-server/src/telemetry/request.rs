//! The per-request HTTP tracing middleware: [`trace_request`].

use std::time::Instant;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use http::header::HeaderValue;
use tracing::field;
use uuid::Uuid;

use crate::state::AppState;

/// The header carrying the per-request correlation id, read from the inbound
/// request (or generated) and echoed on the response.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
