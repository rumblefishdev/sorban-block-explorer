//! Builders and canonical error codes for HTTP `ErrorEnvelope` responses.
//!
//! Every handler in the API returns failures via the flat
//! [`ErrorEnvelope`] shape defined in ADR 0008 and declared in
//! [`crate::openapi::schemas`]. This module provides ergonomic constructors
//! so call sites do not hand-roll the JSON envelope and cannot drift on
//! the shape (e.g. nesting under an `error` key, dropping `details`).
//!
//! The string constants below are the stable machine-readable codes clients
//! key off. Adding new codes is fine; renaming or removing them is a
//! breaking change and requires the same ADR/superseded dance as a schema
//! change.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::openapi::schemas::ErrorEnvelope;

// ---------------------------------------------------------------------------
// Canonical error codes
// ---------------------------------------------------------------------------

/// Pagination cursor failed base64/JSON decode or did not match the
/// expected schema for the endpoint.
pub const INVALID_CURSOR: &str = "invalid_cursor";

/// `limit` query parameter was zero, negative, non-numeric, or above the
/// per-endpoint maximum.
pub const INVALID_LIMIT: &str = "invalid_limit";

/// A `filter[key]` query parameter carried a value the endpoint could
/// not interpret (unknown enum name, malformed StrKey, etc.).
pub const INVALID_FILTER: &str = "invalid_filter";

/// Resource not found by its primary key (hash, ID, …).
pub const NOT_FOUND: &str = "not_found";

/// Unrecoverable database error. Surfaces as HTTP 500; the detailed
/// cause is logged server-side and never returned to the client.
pub const DB_ERROR: &str = "db_error";

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// Build a 400 Bad Request response with an [`ErrorEnvelope`] body.
///
/// Preferred helper for parameter validation failures in handlers and
/// extractors — the status code is the single most common error branch
/// in this API.
pub fn bad_request(code: &str, message: impl Into<String>) -> Response {
    envelope(StatusCode::BAD_REQUEST, code, message, None)
}

/// Build a 400 response with a `details` payload.
///
/// Use this when the client needs structured context (field name, allowed
/// values, received value, …) to correct the request.
pub fn bad_request_with_details(
    code: &str,
    message: impl Into<String>,
    details: serde_json::Value,
) -> Response {
    envelope(StatusCode::BAD_REQUEST, code, message, Some(details))
}

/// Build a 404 Not Found response.
pub fn not_found(message: impl Into<String>) -> Response {
    envelope(StatusCode::NOT_FOUND, NOT_FOUND, message, None)
}

/// Build a 500 Internal Server Error response.
///
/// The generic `message` is safe to return to clients; the actual cause
/// should be logged separately at the call site before invoking this.
pub fn internal_error(code: &str, message: impl Into<String>) -> Response {
    envelope(StatusCode::INTERNAL_SERVER_ERROR, code, message, None)
}

/// Low-level envelope builder. Most handlers prefer the status-specific
/// helpers above.
pub fn envelope(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> Response {
    (
        status,
        Json(ErrorEnvelope {
            code: code.to_string(),
            message: message.into(),
            details,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body;

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn bad_request_produces_flat_envelope() {
        let resp = bad_request(INVALID_LIMIT, "limit must be between 1 and 100");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let json = body_json(resp).await;
        assert_eq!(json["code"], "invalid_limit");
        assert_eq!(json["message"], "limit must be between 1 and 100");
        assert!(
            json.get("error").is_none(),
            "envelope must be flat, not nested under `error`"
        );
    }

    #[tokio::test]
    async fn bad_request_with_details_serialises_details() {
        let resp = bad_request_with_details(
            INVALID_LIMIT,
            "limit must be between 1 and 100",
            serde_json::json!({ "min": 1, "max": 100, "received": 0 }),
        );
        let json = body_json(resp).await;
        assert_eq!(json["details"]["max"], 100);
    }

    #[tokio::test]
    async fn details_omitted_when_none() {
        let resp = bad_request(INVALID_CURSOR, "cursor is malformed");
        let json = body_json(resp).await;
        assert!(
            json.get("details").is_none(),
            "details should be omitted when None (serde skip_serializing_if): {json}"
        );
    }

    #[tokio::test]
    async fn not_found_uses_canonical_code() {
        let resp = not_found("transaction not found");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let json = body_json(resp).await;
        assert_eq!(json["code"], "not_found");
    }

    #[tokio::test]
    async fn internal_error_uses_500_and_flat_envelope() {
        // Handlers route DB failures through `internal_error(DB_ERROR, ...)`.
        // Lock the contract — 500 status + flat `{ code, message }` shape —
        // so a future "wrap errors under `error` key" change cannot slip
        // through without breaking this test.
        let resp = internal_error(DB_ERROR, "database error");
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = body_json(resp).await;
        assert_eq!(json["code"], "db_error");
        assert_eq!(json["message"], "database error");
        assert!(
            json.get("error").is_none(),
            "envelope must be flat, not nested under `error`"
        );
    }
}
