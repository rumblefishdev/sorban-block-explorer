//! axum extractors for the standard `?limit=&cursor=` query parameters.
//!
//! [`Pagination<P>`] is a handler argument extractor that reads `limit`
//! and `cursor` from the query string (tolerating unknown fields so it
//! composes with a sibling `Query<FilterParams>`), validates them, and
//! returns a `limit: u32` plus an optional decoded cursor payload.
//!
//! Validation failures surface the canonical `ErrorEnvelope` codes
//! (`invalid_limit`, `invalid_cursor`).

#![allow(clippy::result_large_err)]

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use axum::response::Response;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::cursor::{self, CursorError};
use super::errors;

/// Per-endpoint limit policy. Both `default` and `max` are `u32` to match
/// the wire type; `default` must satisfy `1 <= default <= max`.
#[derive(Debug, Clone, Copy)]
pub struct LimitConfig {
    pub default: u32,
    pub max: u32,
}

impl LimitConfig {
    /// Project default — 20 items per page, 100 item ceiling (matches ADR
    /// 0008 guidance and current spec for every list endpoint).
    pub const DEFAULT: LimitConfig = LimitConfig {
        default: 20,
        max: 100,
    };
}

impl Default for LimitConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Raw deserialisation target for the two standard query parameters.
///
/// Uses `Option<String>` for `limit` (not `Option<u32>`) so non-numeric
/// values fall into our validator with an `INVALID_LIMIT` response rather
/// than being rejected by serde with a generic 422.
#[derive(Debug, Default, Deserialize)]
struct PaginationRaw {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
}

/// Validated pagination parameters with a decoded cursor payload.
///
/// Generic over `P` — the resource-specific cursor payload. Use
/// [`cursor::TsIdCursor`](super::cursor::TsIdCursor) for the common
/// `(created_at, id)` case.
#[derive(Debug)]
pub struct Pagination<P> {
    pub limit: u32,
    pub cursor: Option<P>,
}

impl<P: DeserializeOwned> Pagination<P> {
    /// Validate a raw `?limit=&cursor=` pair using the project-default
    /// [`LimitConfig::DEFAULT`].
    fn resolve_default(limit: Option<&str>, cursor: Option<&str>) -> Result<Self, Response> {
        let limit = validate_limit(limit, LimitConfig::DEFAULT)?;
        let cursor = decode_cursor::<P>(cursor)?;
        Ok(Pagination { limit, cursor })
    }
}

// ---------------------------------------------------------------------------
// Validation primitives (also used by the FromRequestParts impl)
// ---------------------------------------------------------------------------

fn validate_limit(raw: Option<&str>, cfg: LimitConfig) -> Result<u32, Response> {
    let Some(s) = raw else {
        return Ok(cfg.default);
    };

    let parsed: u32 = s.parse().map_err(|_| {
        errors::bad_request_with_details(
            errors::INVALID_LIMIT,
            format!("limit must be an integer between 1 and {}", cfg.max),
            serde_json::json!({ "min": 1, "max": cfg.max, "received": s }),
        )
    })?;

    if parsed == 0 || parsed > cfg.max {
        return Err(errors::bad_request_with_details(
            errors::INVALID_LIMIT,
            format!("limit must be between 1 and {}", cfg.max),
            serde_json::json!({ "min": 1, "max": cfg.max, "received": parsed }),
        ));
    }

    Ok(parsed)
}

fn decode_cursor<P: DeserializeOwned>(raw: Option<&str>) -> Result<Option<P>, Response> {
    let Some(s) = raw else {
        return Ok(None);
    };

    match cursor::decode::<P>(s) {
        Ok(p) => Ok(Some(p)),
        Err(CursorError::InvalidBase64) | Err(CursorError::InvalidPayload) => Err(
            errors::bad_request(errors::INVALID_CURSOR, "cursor is malformed or expired"),
        ),
    }
}

// ---------------------------------------------------------------------------
// FromRequestParts impl
// ---------------------------------------------------------------------------

/// Extractor impl uses [`LimitConfig::DEFAULT`].
///
/// Internally delegates to `axum::extract::Query<PaginationRaw>`, which
/// tolerates unknown fields in the query string — so a handler can pair
/// this extractor with a sibling `Query<FilterParams>` carrying the
/// `filter[...]` entries without conflict.
impl<S, P> FromRequestParts<S> for Pagination<P>
where
    S: Send + Sync,
    P: DeserializeOwned,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(raw) = Query::<PaginationRaw>::from_request_parts(parts, state)
            .await
            .map_err(|e| {
                errors::bad_request(
                    errors::INVALID_LIMIT,
                    format!("could not parse pagination parameters: {e}"),
                )
            })?;
        Pagination::<P>::resolve_default(raw.limit.as_deref(), raw.cursor.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::cursor::TsIdCursor;
    use axum::body;
    use axum::http::StatusCode;
    use chrono::{TimeZone, Utc};

    async fn body_json(resp: Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    #[test]
    fn limit_default_when_missing() {
        assert_eq!(validate_limit(None, LimitConfig::DEFAULT).unwrap(), 20);
    }

    #[test]
    fn limit_within_bounds_accepted() {
        assert_eq!(
            validate_limit(Some("42"), LimitConfig::DEFAULT).unwrap(),
            42
        );
        assert_eq!(
            validate_limit(Some("100"), LimitConfig::DEFAULT).unwrap(),
            100
        );
        assert_eq!(validate_limit(Some("1"), LimitConfig::DEFAULT).unwrap(), 1);
    }

    #[tokio::test]
    async fn limit_zero_rejected_with_invalid_limit() {
        let err = validate_limit(Some("0"), LimitConfig::DEFAULT).unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_limit");
        assert_eq!(json["details"]["received"], 0);
    }

    #[tokio::test]
    async fn limit_above_max_rejected() {
        let err = validate_limit(Some("101"), LimitConfig::DEFAULT).unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_limit");
        assert_eq!(json["details"]["max"], 100);
    }

    #[tokio::test]
    async fn limit_non_numeric_rejected() {
        let err = validate_limit(Some("many"), LimitConfig::DEFAULT).unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_limit");
        assert_eq!(json["details"]["received"], "many");
    }

    #[test]
    fn cursor_none_when_missing() {
        let result: Option<TsIdCursor> = decode_cursor(None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn cursor_decoded_when_valid() {
        let encoded = cursor::encode(&TsIdCursor::new(
            Utc.with_ymd_and_hms(2026, 4, 24, 12, 0, 0).unwrap(),
            42,
        ));
        let decoded: Option<TsIdCursor> = decode_cursor(Some(&encoded)).unwrap();
        assert_eq!(decoded.unwrap().id, 42);
    }

    #[tokio::test]
    async fn cursor_malformed_rejected_with_invalid_cursor() {
        let err = decode_cursor::<TsIdCursor>(Some("not!!base64")).unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_cursor");
    }

    #[tokio::test]
    async fn cursor_wrong_schema_rejected_with_invalid_cursor() {
        use base64::Engine;
        let bad = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{}");
        let err = decode_cursor::<TsIdCursor>(Some(&bad)).unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_cursor");
    }
}
