//! Validators for typed `filter[key]` query parameters.
//!
//! The `filter[key]=value` convention is project-wide (see ADR 0008 and
//! task 0043) — every list endpoint exposes some subset of keys. The
//! *parsing* of those keys is already handled cleanly by `serde` renames
//! on the endpoint's own `ListParams` DTO; what handlers actually
//! duplicate is the *validation* of each value (StrKey shape, enum name
//! recognition) and the mapping of failures to the canonical
//! `ErrorEnvelope { code: "invalid_filter" }`.
//!
//! This module provides those validators. Each returns `Ok(T)` on success
//! or a fully built 400 [`Response`] on failure, so handlers can
//! `?`-propagate into their list endpoint bodies.

use std::str::FromStr;

use axum::response::Response;

use super::errors;

/// Shape-validate a Stellar StrKey: required prefix character + 56 total
/// chars in the RFC 4648 base32 alphabet (`A-Z` + `2-7`).
///
/// CRC validation is deliberately skipped — the shape check catches the
/// common typo / wrong-prefix cases that would otherwise produce silent
/// empty result sets, without pulling in a CRC dependency. A full CRC
/// check is duplicated inside the DB anyway when the StrKey is resolved
/// against `accounts.account_id` / `soroban_contracts.contract_id`.
pub fn strkey(value: &str, prefix: char, filter_key: &str) -> Result<(), Response> {
    if value.len() == 56
        && value.starts_with(prefix)
        && value.chars().all(|c| matches!(c, 'A'..='Z' | '2'..='7'))
    {
        Ok(())
    } else {
        Err(errors::bad_request_with_details(
            errors::INVALID_FILTER,
            format!(
                "filter[{filter_key}] is not a valid Stellar StrKey (prefix {prefix}, 56 chars, base32)"
            ),
            serde_json::json!({ "filter": filter_key, "received": value, "expected_prefix": prefix.to_string() }),
        ))
    }
}

/// Parse a `filter[key]` string into an enum type via [`FromStr`].
///
/// Wraps the enum's parse error in the canonical `invalid_filter`
/// envelope so handlers do not hand-craft the response. Suitable for any
/// type whose `FromStr` implementation returns an error for unknown
/// variant names — e.g. `domain::OperationType`, `domain::TokenType`.
pub fn parse_enum<T>(value: &str, filter_key: &str) -> Result<T, Response>
where
    T: FromStr,
{
    T::from_str(value).map_err(|_| {
        errors::bad_request_with_details(
            errors::INVALID_FILTER,
            format!("filter[{filter_key}] is not a recognised value"),
            serde_json::json!({ "filter": filter_key, "received": value }),
        )
    })
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

    // Synthetic shape-valid StrKeys: prefix char + 55 body chars = 56 total,
    // all in RFC 4648 base32 alphabet. Not real addresses; CRC is not
    // validated by `strkey()` on purpose.
    const VALID_G: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAT";
    const VALID_C: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJ";

    // Length-check sanity: the StrKey prefix + 55 more chars = 56. The
    // constants above are synthetic shape-valid placeholders, not real
    // addresses — length is asserted explicitly because getting this
    // wrong in a test constant would mask validator bugs.
    #[test]
    fn test_constants_are_56_chars() {
        assert_eq!(VALID_G.len(), 56);
        assert_eq!(VALID_C.len(), 56);
    }

    #[test]
    fn strkey_valid_account_accepted() {
        assert!(strkey(VALID_G, 'G', "source_account").is_ok());
    }

    #[test]
    fn strkey_valid_contract_accepted() {
        assert!(strkey(VALID_C, 'C', "contract_id").is_ok());
    }

    #[tokio::test]
    async fn strkey_wrong_prefix_rejected() {
        let err = strkey(VALID_C, 'G', "source_account").unwrap_err();
        let (status, json) = body_json(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], "invalid_filter");
        assert_eq!(json["details"]["filter"], "source_account");
    }

    #[tokio::test]
    async fn strkey_wrong_length_rejected() {
        let err = strkey("GTOO_SHORT", 'G', "source_account").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_filter");
    }

    #[tokio::test]
    async fn strkey_invalid_alphabet_rejected() {
        // Contains `0` and `1`, which are not in RFC 4648 base32.
        let bad = "G0000000000000000000000000000000000000000000000000001T";
        let err = strkey(bad, 'G', "source_account").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_filter");
    }

    // A tiny standalone enum so we do not couple the unit test to
    // `domain::OperationType`. The real-world use site is still
    // exercised via the retro-refactored transactions handler in Step 8.
    #[derive(Debug, PartialEq)]
    enum Kind {
        Alpha,
        Beta,
    }
    impl FromStr for Kind {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            match s {
                "ALPHA" => Ok(Kind::Alpha),
                "BETA" => Ok(Kind::Beta),
                _ => Err(()),
            }
        }
    }

    #[test]
    fn parse_enum_accepts_known_variant() {
        assert_eq!(parse_enum::<Kind>("ALPHA", "kind").unwrap(), Kind::Alpha);
    }

    #[tokio::test]
    async fn parse_enum_rejects_unknown_variant() {
        let err = parse_enum::<Kind>("GAMMA", "kind").unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_filter");
        assert_eq!(json["details"]["received"], "GAMMA");
    }
}
