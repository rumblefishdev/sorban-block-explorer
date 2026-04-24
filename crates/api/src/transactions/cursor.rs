//! Opaque cursor encoding for the transactions list endpoint.
//!
//! Cursor = base64url(JSON { "ts": "<ISO 8601>", "id": <i64> }).
//! The `ts` field is needed so the DB query can prune partitions via the
//! `created_at` column (without it, the planner would scan all partitions).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct CursorPayload {
    ts: DateTime<Utc>,
    id: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum CursorError {
    #[error("invalid base64")]
    InvalidBase64,
    #[error("invalid cursor payload")]
    InvalidPayload,
}

pub fn encode(ts: DateTime<Utc>, id: i64) -> String {
    let payload = CursorPayload { ts, id };
    let json = serde_json::to_string(&payload).expect("CursorPayload serialization is infallible");
    URL_SAFE_NO_PAD.encode(json.as_bytes())
}

pub fn decode(s: &str) -> Result<(DateTime<Utc>, i64), CursorError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| CursorError::InvalidBase64)?;
    let payload: CursorPayload =
        serde_json::from_slice(&bytes).map_err(|_| CursorError::InvalidPayload)?;
    Ok((payload.ts, payload.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn round_trip() {
        let ts = Utc.with_ymd_and_hms(2026, 4, 23, 12, 0, 0).unwrap();
        let id = 42_000_i64;
        let encoded = encode(ts, id);
        let (decoded_ts, decoded_id) = decode(&encoded).unwrap();
        assert_eq!(decoded_ts, ts);
        assert_eq!(decoded_id, id);
    }

    #[test]
    fn invalid_base64_returns_error() {
        assert!(matches!(
            decode("not!!base64"),
            Err(CursorError::InvalidBase64)
        ));
    }

    #[test]
    fn invalid_payload_returns_error() {
        let bad = URL_SAFE_NO_PAD.encode(b"{}");
        assert!(matches!(decode(&bad), Err(CursorError::InvalidPayload)));
    }
}
