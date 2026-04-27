//! Opaque cursor encoding shared by every paginated list endpoint.
//!
//! The wire format is `base64url(JSON(<payload>))` with no padding. The
//! payload is a resource-specific struct — most resources can use
//! [`TsIdCursor`], which encodes `(created_at, id)` and matches the natural
//! DESC ordering + `id` tie-break of the partitioned fact tables.
//!
//! Cursor opacity is a public-contract requirement from ADR 0008: clients
//! must never construct a cursor by hand or rely on its internal shape.
//! That means the payload type can change between releases without a
//! breaking API change, as long as the previous format fails decode cleanly
//! and produces an `INVALID_CURSOR` error.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Error returned by [`decode`] when a client-supplied cursor string
/// cannot be parsed.
///
/// Both variants map to HTTP 400 + `ErrorEnvelope { code: "invalid_cursor" }`
/// at the extractor boundary. The distinction is kept internal so tests and
/// logs can assert on the specific failure mode without exposing it over
/// the wire.
#[derive(Debug, thiserror::Error)]
pub enum CursorError {
    #[error("cursor is not valid base64url")]
    InvalidBase64,
    #[error("cursor payload does not match expected schema")]
    InvalidPayload,
}

/// Encode a typed payload as a base64url-JSON cursor string.
///
/// Serialisation is infallible for every payload type whose `Serialize`
/// impl is total (all `#[derive(Serialize)]` structs fit this). A panic
/// here would indicate a broken manual `Serialize` impl, not user input,
/// so we surface it as a panic rather than an error.
pub fn encode<P: Serialize>(payload: &P) -> String {
    // to_vec, not to_string — JSON output is already valid UTF-8, so
    // round-tripping through String just adds a redundant validation +
    // allocation. Encoder takes &[u8] anyway.
    let json = serde_json::to_vec(payload).expect("cursor payload serialization is infallible");
    URL_SAFE_NO_PAD.encode(&json)
}

/// Decode a base64url-JSON cursor string into a typed payload.
///
/// Returns [`CursorError::InvalidBase64`] for malformed encoding and
/// [`CursorError::InvalidPayload`] when the decoded bytes are not valid
/// JSON for `P` (wrong schema, truncated, etc.).
pub fn decode<P: DeserializeOwned>(s: &str) -> Result<P, CursorError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| CursorError::InvalidBase64)?;
    serde_json::from_slice(&bytes).map_err(|_| CursorError::InvalidPayload)
}

/// Default cursor payload for resources ordered by `(created_at DESC, id DESC)`.
///
/// Every partitioned fact table in the explorer (transactions, operations,
/// ledgers, events, …) orders its list endpoints this way, so this payload
/// covers the common case. Resources with a different natural ordering
/// define their own payload struct and reuse [`encode`] / [`decode`] directly.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TsIdCursor {
    pub ts: DateTime<Utc>,
    pub id: i64,
}

impl TsIdCursor {
    pub fn new(ts: DateTime<Utc>, id: i64) -> Self {
        Self { ts, id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn ts_id_round_trip() {
        let ts = Utc.with_ymd_and_hms(2026, 4, 24, 12, 0, 0).unwrap();
        let original = TsIdCursor::new(ts, 42_000_i64);
        let encoded = encode(&original);
        let decoded: TsIdCursor = decode(&encoded).unwrap();
        assert_eq!(decoded.ts, original.ts);
        assert_eq!(decoded.id, original.id);
    }

    #[test]
    fn invalid_base64_returns_invalid_base64() {
        let err = decode::<TsIdCursor>("not!!base64").unwrap_err();
        assert!(matches!(err, CursorError::InvalidBase64));
    }

    #[test]
    fn wrong_schema_returns_invalid_payload() {
        let bad = URL_SAFE_NO_PAD.encode(b"{}");
        let err = decode::<TsIdCursor>(&bad).unwrap_err();
        assert!(matches!(err, CursorError::InvalidPayload));
    }

    #[test]
    fn truncated_json_returns_invalid_payload() {
        let bad = URL_SAFE_NO_PAD.encode(br#"{"ts":"2026-04-24T12:00:00Z""#);
        let err = decode::<TsIdCursor>(&bad).unwrap_err();
        assert!(matches!(err, CursorError::InvalidPayload));
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
    struct SeqCursor {
        seq: u64,
    }

    #[test]
    fn custom_payload_round_trip() {
        let original = SeqCursor { seq: 12_345 };
        let encoded = encode(&original);
        let decoded: SeqCursor = decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }
}
