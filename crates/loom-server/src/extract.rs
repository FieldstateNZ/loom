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
//!
//! [`TurnJson`] reads the stateful turns route, whose larger limit applies
//! only to private memory turns, the only turns that may carry photos.

use axum::body::{Body, Bytes};
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use http::StatusCode;

use crate::error::ApiError;
use crate::v1::TurnRequest;

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

/// The turns route's request body. The route's 5 MiB layer admits photos on a
/// private memory turn; every other turn keeps axum's 2 MiB default.
///
/// - A body over the route's limit renders as `413 payload_too_large`.
/// - Within 2 MiB, parsing and errors are exactly those of [`Json`].
/// - Above 2 MiB only a [`TurnRequest`] with `gateway_mcp` is accepted; any
///   other JSON body gets the `400` an over-limit body always got, and a body
///   not sent as JSON keeps its `415`.
#[derive(Debug)]
pub struct TurnJson(pub TurnRequest);

impl<S: Send + Sync> FromRequest<S> for TurnJson {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (parts, body) = req.into_parts();
        let headers = parts.headers.clone();
        let bytes = Bytes::from_request(Request::from_parts(parts, body), state)
            .await
            .map_err(|rejection| {
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::new(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "payload_too_large",
                        "request body is too large",
                    )
                } else {
                    ApiError::bad_request("request body could not be read")
                }
            })?;
        // The buffered copy carries no route limit, so axum's 2 MiB default
        // reads it exactly as it read every turn before the limit was raised.
        let mut buffered = Request::new(Body::from(bytes.clone()));
        *buffered.headers_mut() = headers;
        match axum::Json::<TurnRequest>::from_request(buffered, state).await {
            Ok(axum::Json(turn)) => Ok(Self(turn)),
            Err(JsonRejection::BytesRejection(_)) => {
                match axum::Json::<TurnRequest>::from_bytes(&bytes) {
                    Ok(axum::Json(turn)) if turn.gateway_mcp.is_some() => Ok(Self(turn)),
                    _ => Err(ApiError::bad_request("request body could not be read")),
                }
            }
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
