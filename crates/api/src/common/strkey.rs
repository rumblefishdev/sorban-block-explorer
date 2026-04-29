//! Shared primitive: Stellar StrKey shape check.
//!
//! Lives at the `common::*` layer (not under `filters` or `path`) because
//! both consumers — query-string filter validators ([`crate::common::filters`])
//! and URL path validators ([`crate::common::path`]) — need the same check
//! but emit different envelope codes (`invalid_filter` vs
//! `invalid_contract_id` / `invalid_account_id`). Keeping the shape check
//! here avoids a peer module depending on another peer module purely for
//! a five-line helper.
//!
//! The shape rule is a verbatim port of the Stellar StrKey grammar
//! restricted to the prefix + body + length checks (CRC validation is
//! intentionally omitted — see [`is_strkey_shape`] doc).

/// Returns `true` iff `value` is exactly 56 characters, starts with
/// `prefix`, and every byte is in the RFC 4648 base32 alphabet
/// (`A-Z` and `2-7`).
///
/// `bytes()` (not `chars()`) — base32 is ASCII-only, so byte iteration
/// is safe and skips the UTF-8 decode.
///
/// `prefix` is enforced strictly: a value that passes the alphabet +
/// length checks but starts with the wrong prefix character is rejected.
/// This is what stops a `G…` account StrKey from sneaking through a
/// contract-id validator (the alphabet check alone would accept it
/// because `G` is in `A-Z`).
///
/// **CRC is not validated** — the shape check IS the validation, not a
/// fast path before a stricter step. Per ADR 0037 the relevant DB
/// columns (`accounts.account_id`, `soroban_contracts.contract_id`) are
/// `VARCHAR(56) NOT NULL UNIQUE` matched by plain string equality; a
/// wrong-CRC StrKey that passes the shape check simply fails to match
/// any row, producing the same UX as a non-existent address. The shape
/// check exists to catch the common typo / wrong-prefix / wrong-alphabet
/// cases loudly with a 400 envelope instead of silently returning `[]`
/// or 404 on a junk address.
pub(crate) fn is_strkey_shape(value: &str, prefix: char) -> bool {
    value.len() == 56
        && value.starts_with(prefix)
        && value
            .bytes()
            .all(|b| matches!(b, b'A'..=b'Z' | b'2'..=b'7'))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic shape-valid placeholders (no CRC), 56 chars each.
    const VALID_C: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJ";
    const VALID_G: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAT";

    #[test]
    fn accepts_correct_prefix() {
        assert!(is_strkey_shape(VALID_C, 'C'));
        assert!(is_strkey_shape(VALID_G, 'G'));
    }

    #[test]
    fn rejects_wrong_prefix() {
        // Value is well-formed StrKey but for the OTHER prefix — must reject.
        // This is the security-critical branch: without it, an account
        // address would slip through a contract-id check.
        assert!(!is_strkey_shape(VALID_G, 'C'));
        assert!(!is_strkey_shape(VALID_C, 'G'));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!is_strkey_shape("CAAA", 'C'));
        let too_long = format!("C{}", "A".repeat(60)); // 61 chars
        assert!(!is_strkey_shape(&too_long, 'C'));
        assert!(!is_strkey_shape("", 'C'));
    }

    #[test]
    fn rejects_invalid_alphabet() {
        // Contains `0` (not in base32). Length 56, prefix C — only alphabet fails.
        let bad = "C00000000000000000000000000000000000000000000000000000A";
        assert!(!is_strkey_shape(bad, 'C'));
    }

    #[test]
    fn rejects_lowercase() {
        // Lowercase 'a' is outside the uppercase-only base32 subset Stellar uses.
        let bad = "Caaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert!(!is_strkey_shape(bad, 'C'));
    }
}
