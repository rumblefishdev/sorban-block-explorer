use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Internal cursor payload — never exposed to clients.
/// For descending pagination: `before_sequence` is the last seen sequence;
/// the next page fetches rows with `sequence < before_sequence`.
#[derive(Debug, Serialize, Deserialize)]
struct CursorPayload {
    before_sequence: i64,
}

pub fn encode(last_seen_sequence: i64) -> String {
    let payload = CursorPayload {
        before_sequence: last_seen_sequence,
    };
    let json = serde_json::to_vec(&payload).expect("cursor serialization");
    URL_SAFE_NO_PAD.encode(&json)
}

pub fn decode(cursor: &str) -> Result<i64, AppError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| AppError::BadRequest("invalid cursor".into()))?;
    let payload: CursorPayload = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::BadRequest("invalid cursor".into()))?;
    Ok(payload.before_sequence)
}
