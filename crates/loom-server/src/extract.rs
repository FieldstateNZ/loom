//! A JSON request-body extractor that renders rejections through [`ApiError`].
//!
//! axum's built-in [`axum::Json`] extractor rejects a malformed or wrong-shape
//! request body with a bare `text/plain` response, bypassing the gateway's
//! shared `{ "error": { code, message } }` envelope. This [`Json`] wrapper
//! delegates the actual parsing to [`axum::Json`] but maps every
//! [`JsonRejection`] onto an [`ApiError`], so a client always sees the same
//! enveloped error shape regardless of where the request failed.
//!
//! Use it only in request-body position; response-position serialisation should
//! keep using [`axum::Json`].

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use http::StatusCode;

use crate::error::ApiError;

/// A drop-in replacement for [`axum::Json`] in request-body position that maps
/// extraction failures onto the shared [`ApiError`] envelope.
///
/// - Invalid JSON syntax renders as `400 bad_request`.
/// - Well-formed JSON of the wrong shape (a missing or mismatched field) renders
///   as `422 unprocessable`.
///
/// The inner value is exposed as the tuple field, so handlers destructure it
/// exactly like `axum::Json`: `Json(req): Json<MyBody>`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    axum::Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(map_rejection(&rejection)),
        }
    }
}

/// Maps an axum [`JsonRejection`] onto the shared [`ApiError`] envelope.
///
/// Messages are fixed and non-sensitive: the client's own body (which an
/// attacker controls) is never echoed back.
fn map_rejection(rejection: &JsonRejection) -> ApiError {
    match rejection {
        // The bytes were not valid JSON at all.
        JsonRejection::JsonSyntaxError(_) => {
            ApiError::bad_request("request body is not valid JSON")
        }
        // Valid JSON, but the wrong shape (a missing or mismatched field).
        JsonRejection::JsonDataError(_) => ApiError::unprocessable(
            "unprocessable_entity",
            "request body does not match the expected schema",
        ),
        // The request was not declared as `application/json`.
        JsonRejection::MissingJsonContentType(_) => ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "request body must be sent as application/json",
        ),
        // Body could not be buffered, or a future non-exhaustive variant.
        _ => ApiError::bad_request("request body could not be read"),
    }
}
