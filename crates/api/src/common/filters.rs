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

#![allow(clippy::result_large_err)]

use std::str::FromStr;

use axum::response::Response;

use super::errors;

/// Shape-validate a Stellar StrKey: required prefix character + 56 total
/// chars in the RFC 4648 base32 alphabet (`A-Z` + `2-7`).
///
/// CRC validation is deliberately skipped — the shape check this function
/// performs IS the validation, not a fast path before a stricter DB check.
/// Per ADR 0037, `accounts.account_id` and `soroban_contracts.contract_id`
/// are `VARCHAR(56) NOT NULL UNIQUE` columns matched by plain string
/// equality; a wrong-CRC StrKey that passes the shape check will simply
/// not match any row, producing an empty result set with the same UX as
/// a non-existent address — acceptable for a read-only API. The benefit
/// of the shape check is catching the common typo / wrong-prefix cases
/// loudly with a 400 envelope instead of silently returning `[]`.
pub fn strkey(value: &str, prefix: char, filter_key: &str) -> Result<(), Response> {
    // bytes() instead of chars() — StrKey alphabet is RFC 4648 base32 (ASCII
    // only), so byte iteration is safe and skips the UTF-8 decode.
    if value.len() == 56
        && value.starts_with(prefix)
        && value
            .bytes()
            .all(|b| matches!(b, b'A'..=b'Z' | b'2'..=b'7'))
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

/// Validate a StrKey only when present.
///
/// Common handler pattern — `Option<String>` filter param, validate only
/// when client supplied a value. Saves the `if let Some(v) = ... && let
/// Err(resp) = ...` let-chain at every call site.
pub fn strkey_opt(value: Option<&str>, prefix: char, filter_key: &str) -> Result<(), Response> {
    match value {
        Some(v) => strkey(v, prefix, filter_key),
        None => Ok(()),
    }
}

/// Parse a `filter[key]` string into an enum type via [`FromStr`].
///
/// Wraps the enum's parse error in the canonical `invalid_filter`
/// envelope so handlers do not hand-craft the response. Suitable for any
/// type whose `FromStr` implementation returns an error for unknown
/// variant names — e.g. `domain::OperationType`, `domain::TokenType`.
///
/// `kind_hint` lets the call site tighten the error message with a
/// type-specific noun (e.g. `Some("operation type")` →
/// *"filter[operation_type] is not a recognized operation type"*).
/// Pass `None` for the generic *"is not a recognized value"* phrasing.
pub fn parse_enum<T>(value: &str, filter_key: &str, kind_hint: Option<&str>) -> Result<T, Response>
where
    T: FromStr,
{
    T::from_str(value).map_err(|_| {
        let what = kind_hint.unwrap_or("value");
        errors::bad_request_with_details(
            errors::INVALID_FILTER,
            format!("filter[{filter_key}] is not a recognized {what}"),
            serde_json::json!({ "filter": filter_key, "received": value }),
        )
    })
}

/// Parse a `filter[key]` enum only when present. See [`strkey_opt`] for the
/// rationale — symmetric helper for the enum case.
pub fn parse_enum_opt<T>(
    value: Option<&str>,
    filter_key: &str,
    kind_hint: Option<&str>,
) -> Result<Option<T>, Response>
where
    T: FromStr,
{
    value
        .map(|s| parse_enum::<T>(s, filter_key, kind_hint))
        .transpose()
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
        assert_eq!(
            parse_enum::<Kind>("ALPHA", "kind", None).unwrap(),
            Kind::Alpha
        );
    }

    #[tokio::test]
    async fn parse_enum_rejects_unknown_variant_with_generic_hint() {
        let err = parse_enum::<Kind>("GAMMA", "kind", None).unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(json["code"], "invalid_filter");
        assert_eq!(json["details"]["received"], "GAMMA");
        assert_eq!(json["message"], "filter[kind] is not a recognized value");
    }

    #[tokio::test]
    async fn parse_enum_rejects_unknown_variant_with_kind_hint() {
        let err = parse_enum::<Kind>("GAMMA", "kind", Some("kind name")).unwrap_err();
        let (_, json) = body_json(err).await;
        assert_eq!(
            json["message"],
            "filter[kind] is not a recognized kind name"
        );
    }
}
