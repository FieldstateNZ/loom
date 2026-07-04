//! The structured HTTP error envelope shared by every endpoint.
//!
//! All errors serialise to `{ "error": { "code", "message", "provider_error"? } }`
//! so clients can branch on a stable machine-readable `code` rather than parse
//! prose. Internal faults are logged with their cause but never leak details
//! (connection strings, SQL, secrets) into the response body.

use axum::response::{IntoResponse, Response};
use axum::Json;
use http::StatusCode;
use serde::Serialize;

use crate::crypto::CryptoError;

/// A structured API error that renders as the standard error envelope.
#[derive(Debug)]
pub struct ApiError {
    /// The HTTP status code.
    status: StatusCode,
    /// A stable, machine-readable error code (e.g. `"unauthorized"`).
    code: &'static str,
    /// A human-readable, non-sensitive message.
    message: String,
    /// An optional verbatim provider-native error payload.
    provider_error: Option<serde_json::Value>,
}

impl ApiError {
    /// Builds an error with an explicit status and code.
    #[must_use]
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            provider_error: None,
        }
    }

    /// Attaches a provider-native error payload to the envelope.
    #[must_use]
    pub fn with_provider_error(mut self, payload: serde_json::Value) -> Self {
        self.provider_error = Some(payload);
        self
    }

    /// A `401 Unauthorized` (authentication missing, malformed, or rejected).
    #[must_use]
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    /// A `400 Bad Request` (malformed or invalid input).
    #[must_use]
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    /// A `404 Not Found`.
    #[must_use]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// A `409 Conflict` (e.g. a uniqueness violation).
    #[must_use]
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    /// A `503 Service Unavailable` (a dependency is not ready).
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "unavailable", message)
    }

    /// A generic `500 Internal Server Error` with a fixed, non-sensitive body.
    #[must_use]
    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "an internal error occurred",
        )
    }

    /// Maps a store error to an API error, translating a unique-constraint
    /// violation to `409 Conflict` and logging (but not leaking) anything else.
    #[must_use]
    pub fn from_store(err: loom_store::StoreError) -> Self {
        if is_unique_violation(&err) {
            return Self::conflict("a resource with the same unique key already exists");
        }
        tracing::error!(error = %err, "store operation failed");
        Self::internal()
    }
}

/// Returns `true` if a store error is a PostgreSQL unique-constraint violation.
fn is_unique_violation(err: &loom_store::StoreError) -> bool {
    if let loom_store::StoreError::Database(sqlx::Error::Database(db)) = err {
        return db.code().as_deref() == Some("23505");
    }
    false
}

impl From<CryptoError> for ApiError {
    fn from(err: CryptoError) -> Self {
        tracing::error!(error = %err, "credential encryption failed");
        ApiError::internal()
    }
}

/// The serialised envelope body: `{ "error": { ... } }`.
#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

/// The inner error object.
#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_error: Option<&'a serde_json::Value>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorEnvelope {
            error: ErrorBody {
                code: self.code,
                message: &self.message,
                provider_error: self.provider_error.as_ref(),
            },
        };
        (self.status, Json(body)).into_response()
    }
}
