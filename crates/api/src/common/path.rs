//! Validators for typed path parameters (`/v1/<resource>/:id`).
//!
//! The `:id` placeholder takes different shapes per resource:
//!
//!   | Resource          | Shape                          | Helper                  |
//!   | ----------------- | ------------------------------ | ----------------------- |
//!   | `transactions`    | 64-char hex                    | [`parse_hash`]          |
//!   | `contracts`       | StrKey, prefix `C`             | [`strkey`] with `'C'`   |
//!   | `accounts`        | StrKey, prefix `G`             | [`strkey`] with `'G'`   |
//!   | `liquidity-pools` | 64-char lowercase hex (BYTEA32)| [`pool_id_hex`]         |
//!   | `ledgers`         | numeric `u32`                  | [`sequence`]            |
//!
//! Each helper short-circuits the handler before any DB / S3 call —
//! malformed input maps to a flat ADR 0008 `ErrorEnvelope` with one of
//! the canonical `INVALID_HASH` / `INVALID_CONTRACT_ID` /
//! `INVALID_ACCOUNT_ID` / `INVALID_POOL_ID` / `INVALID_SEQUENCE` codes
//! (see [`crate::common::errors`]).
//!
//! Why not just reuse [`crate::common::filters::strkey`]? `filters::*`
//! emits `invalid_filter` and assumes the value came from a `filter[key]=`
//! query parameter (the error `details` carry a `"filter"` field).
//! Path params have a different code surface and a different `details`
//! shape (`"param"` not `"filter"`) — the helpers below mirror the
//! `filters::*` validation logic but emit path-appropriate envelopes so
//! a client reading the `code` knows immediately whether the bad value
//! came from the URL path or a query string.

#![allow(clippy::result_large_err)]

use axum::response::Response;

use super::errors;
use super::strkey::is_strkey_shape;

// ---------------------------------------------------------------------------
// Hash (transactions)
// ---------------------------------------------------------------------------

/// Validate a transaction-hash path parameter and return the lowercase
/// canonical form for downstream DB / archive lookup.
///
/// Stellar transaction hashes are SHA-256 outputs serialised as 64
/// lowercase or uppercase hex characters. The validator accepts either
/// case so clients can use whatever their indexer / explorer surfaced,
/// and returns the lowercase form on success — the `transactions::hash`
/// column stores lowercase, and downstream archive matching is
/// case-sensitive, so coupling normalisation with validation keeps a
/// future caller from silently 404-ing on uppercase input.
pub fn parse_hash(value: &str) -> Result<String, Response> {
    if value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(value.to_ascii_lowercase())
    } else {
        Err(errors::bad_request_with_details(
            errors::INVALID_HASH,
            "hash must be a 64-character hexadecimal string",
            serde_json::json!({ "param": "hash", "received": value }),
        ))
    }
}

// ---------------------------------------------------------------------------
// StrKey (contracts, accounts)
// ---------------------------------------------------------------------------

/// Validate a Stellar StrKey path parameter (account `G…`, contract `C…`).
///
/// Same shape rule as [`crate::common::filters::strkey`] (RFC 4648 base32,
/// 56 chars, required prefix), but the failure envelope carries the
/// path-specific code (`INVALID_CONTRACT_ID` for `'C'`, `INVALID_ACCOUNT_ID`
/// for `'G'`) and a `"param"` field in `details` instead of `"filter"`.
///
/// CRC validation is skipped — wrong-CRC StrKey passes shape and falls
/// through to DB lookup, which returns `Ok(None)` → 404 `not_found`. UX
/// is identical to a non-existent resource (`/contracts/CCAB...XYZ`
/// where the address is well-formed but never indexed). The shape check
/// catches the common case of a typo / wrong prefix / wrong alphabet
/// loudly with a 400 envelope instead of silently returning 404 on a
/// junk address.
pub fn strkey(value: &str, prefix: char, param: &str) -> Result<(), Response> {
    let code = match prefix {
        'C' => errors::INVALID_CONTRACT_ID,
        'G' => errors::INVALID_ACCOUNT_ID,
        // Future-proofing: any other prefix (M for muxed, T for pre-auth, …)
        // is a path-parameter validation failure, so route through
        // `INVALID_ID` rather than `INVALID_FILTER` (which is reserved for
        // `filter[...]` query params). Add a dedicated const here if a
        // real consumer appears.
        _ => errors::INVALID_ID,
    };

    if is_strkey_shape(value, prefix) {
        Ok(())
    } else {
        Err(errors::bad_request_with_details(
            code,
            format!(
                "{param} must be a 56-character Stellar StrKey starting with '{prefix}' (RFC 4648 base32)"
            ),
            serde_json::json!({ "param": param, "received": value, "expected_prefix": prefix.to_string() }),
        ))
    }
}

// ---------------------------------------------------------------------------
// BYTEA(32) hex (liquidity pools)
// ---------------------------------------------------------------------------

/// Validate a `pool_id`-shaped path parameter.
///
/// LP `pool_id` is `BYTEA(32)` per ADR 0024; the API surfaces it as 64
/// lowercase hex characters. Failure envelope carries `INVALID_POOL_ID`
/// and a `param` field in `details` mirroring the other path validators.
pub fn pool_id_hex(value: &str, param: &str) -> Result<(), Response> {
    if value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(errors::bad_request_with_details(
            errors::INVALID_POOL_ID,
            "pool_id must be a 64-character lowercase hex string",
            serde_json::json!({ "param": param, "received": value }),
        ))
    }
}

// ---------------------------------------------------------------------------
// Sequence (ledgers)
// ---------------------------------------------------------------------------

/// Validate a ledger-sequence path parameter.
///
/// Stellar ledger sequences are monotonically-increasing `u32` values
/// starting at 1 (ledger 0 does not exist in Stellar — genesis is
/// sequence 1; see Stellar Core's `LedgerHeader.ledgerSeq: uint32`).
/// The network has been below 2^32 since genesis and is expected to
/// stay there for the lifetime of this codebase. Zero / negative /
/// non-numeric / overflow inputs map to 400 `INVALID_SEQUENCE`.
///
/// Reserved for the `/v1/ledgers/:sequence` endpoint shipped by task 0047;
/// declared here alongside the other path validators so the canonical
/// set lives in one module.
#[allow(dead_code)]
pub fn sequence(value: &str) -> Result<u32, Response> {
    let invalid = || {
        errors::bad_request_with_details(
            errors::INVALID_SEQUENCE,
            "sequence must be a positive integer that fits in 32 bits",
            serde_json::json!({ "param": "sequence", "received": value }),
        )
    };
    let n = value.parse::<u32>().map_err(|_| invalid())?;
    (n != 0).then_some(n).ok_or_else(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body;
    use axum::http::StatusCode;

    async fn body_json(resp: Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    // -----------------------------------------------------------------------
    // hash
    // -----------------------------------------------------------------------

    #[test]
    fn hash_valid_lowercase_accepted() {
        let h = "ab".repeat(32); // 64 chars, all hex
        assert_eq!(parse_hash(&h).unwrap(), h);
    }

    #[test]
    fn hash_valid_uppercase_normalised_to_lowercase() {
        let h = "AB".repeat(32);
        assert_eq!(parse_hash(&h).unwrap(), "ab".repeat(32));
    }

    #[test]
    fn hash_valid_mixed_case_normalised_to_lowercase() {
        let h = "aB".repeat(32);
        assert_eq!(parse_hash(&h).unwrap(), "ab".repeat(32));
    }

    #[tokio::test]
    async fn hash_wrong_length_rejected_with_invalid_hash() {
        let err = parse_hash("abcdef").unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_hash");
        assert_eq!(json["details"]["param"], "hash");
        assert_eq!(json["details"]["received"], "abcdef");
    }

    #[tokio::test]
    async fn hash_non_hex_char_rejected() {
        let mut h = "ab".repeat(31); // 62 chars
        h.push_str("XX"); // 64 total, X not hex
        let err = parse_hash(&h).unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_hash");
    }

    #[tokio::test]
    async fn hash_empty_rejected() {
        let err = parse_hash("").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_hash");
    }

    // -----------------------------------------------------------------------
    // strkey
    // -----------------------------------------------------------------------

    // Synthetic shape-valid StrKeys (no CRC): prefix + 55 base32 chars = 56.
    const VALID_C: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJ";
    const VALID_G: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAT";

    #[test]
    fn strkey_contract_accepted() {
        assert!(strkey(VALID_C, 'C', "contract_id").is_ok());
    }

    #[test]
    fn strkey_account_accepted() {
        assert!(strkey(VALID_G, 'G', "account_id").is_ok());
    }

    #[tokio::test]
    async fn strkey_contract_wrong_prefix_uses_invalid_contract_id() {
        // Wrong prefix `G` against helper expecting `C` → contract code
        let err = strkey(VALID_G, 'C', "contract_id").unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_contract_id");
        assert_eq!(json["details"]["param"], "contract_id");
        assert_eq!(json["details"]["expected_prefix"], "C");
    }

    #[tokio::test]
    async fn strkey_account_wrong_prefix_uses_invalid_account_id() {
        let err = strkey(VALID_C, 'G', "account_id").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_account_id");
        assert_eq!(json["details"]["expected_prefix"], "G");
    }

    #[tokio::test]
    async fn strkey_wrong_length_rejected() {
        let err = strkey("CTOO_SHORT", 'C', "contract_id").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_contract_id");
    }

    #[tokio::test]
    async fn strkey_invalid_alphabet_rejected() {
        // Keep the correct 56-character shape and `C` prefix, but inject `0`
        // (not in the RFC 4648 base32 alphabet) so the failure is unambiguously
        // an alphabet violation rather than a length mismatch.
        let mut bad = VALID_C.to_string();
        bad.replace_range(1..2, "0");
        assert_eq!(bad.len(), 56);
        let err = strkey(&bad, 'C', "contract_id").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_contract_id");
    }

    // -----------------------------------------------------------------------
    // pool_id_hex
    // -----------------------------------------------------------------------

    #[test]
    fn pool_id_hex_valid_lowercase_accepted() {
        let hex = "ab".repeat(32); // 64 chars all lowercase hex
        assert!(pool_id_hex(&hex, "pool_id").is_ok());
        assert!(pool_id_hex(&"0".repeat(64), "pool_id").is_ok());
    }

    #[tokio::test]
    async fn pool_id_hex_uppercase_rejected() {
        // BYTEA(32) hex on the wire is canonically lowercase; uppercase
        // would round-trip differently through `encode(... 'hex')`.
        let bad = "AB".repeat(32);
        let err = pool_id_hex(&bad, "pool_id").unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_pool_id");
        assert_eq!(json["details"]["param"], "pool_id");
    }

    #[tokio::test]
    async fn pool_id_hex_wrong_length_rejected() {
        let err = pool_id_hex("abcdef", "pool_id").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_pool_id");
    }

    #[tokio::test]
    async fn pool_id_hex_non_hex_char_rejected() {
        // Length 64, lowercase, but contains `g` which is outside hex.
        let mut bad = "a".repeat(63);
        bad.push('g');
        let err = pool_id_hex(&bad, "pool_id").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_pool_id");
    }

    #[tokio::test]
    async fn pool_id_hex_empty_rejected() {
        let err = pool_id_hex("", "pool_id").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_pool_id");
    }

    // -----------------------------------------------------------------------
    // sequence
    // -----------------------------------------------------------------------

    #[test]
    fn sequence_valid_accepted() {
        assert_eq!(sequence("1").unwrap(), 1);
        assert_eq!(sequence("12345678").unwrap(), 12_345_678);
        assert_eq!(sequence("4294967295").unwrap(), u32::MAX);
    }

    #[tokio::test]
    async fn sequence_zero_rejected() {
        // Stellar ledger 0 does not exist — genesis is sequence 1.
        let err = sequence("0").unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_sequence");
        assert_eq!(json["details"]["received"], "0");
    }

    #[tokio::test]
    async fn sequence_negative_rejected() {
        let err = sequence("-1").unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_sequence");
        assert_eq!(json["details"]["received"], "-1");
    }

    #[tokio::test]
    async fn sequence_overflow_rejected() {
        // u32::MAX + 1 = 4294967296 — overflows.
        let err = sequence("4294967296").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_sequence");
    }

    #[tokio::test]
    async fn sequence_non_numeric_rejected() {
        let err = sequence("abc").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_sequence");
    }
}
