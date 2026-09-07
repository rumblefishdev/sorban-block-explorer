//! Query classifier: maps raw `q` to the `(hash_bytes, strkey_prefix)`
//! pair consumed by `22_get_search.sql`.
//!
//! Two derived inputs only — the SQL itself decides which CTE branches
//! fire based on which input is non-NULL. Keeping the classifier this
//! narrow means there is no per-entity dispatch logic in Rust to drift
//! from the SQL contract.

/// Classifier output. `None` means "this branch should not fire".
#[derive(Debug, Default, Clone)]
pub struct Classified {
    /// 32 bytes if `q` parses as 64-char hex or as a full CAP-38/SEP-23
    /// `L…` pool strkey; drives the `transaction` and `pool` exact-match
    /// CTEs (same BYTEA(32) shape on the wire).
    pub hash_bytes: Option<Vec<u8>>,
    /// Upper-cased StrKey or its prefix when `q` matches Stellar StrKey
    /// shape (full 56 chars or any prefix of `G…` / `C…`); drives the
    /// `account` and `contract` prefix CTEs.
    pub strkey_prefix: Option<String>,
    /// `(asset_code, issuer G-StrKey)` when `q` is a fully-qualified asset —
    /// `CODE:ISSUER` (the canonical SEP / SDK form) or `CODE-ISSUER` (the shape
    /// our own `/assets/:id` routes emit, so users paste it back). Drives an
    /// exact asset lookup instead of the code substring scan.
    pub code_issuer: Option<(String, String)>,
}

/// Classify a trimmed, non-empty `q`.
pub fn classify(q: &str) -> Classified {
    let mut out = Classified::default();

    // 32-byte hex (64 chars). Try this first — it is the highest-
    // information shape and unambiguous.
    if q.len() == 64
        && let Ok(bytes) = hex::decode(q)
    {
        out.hash_bytes = Some(bytes);
        return out;
    }

    // Full CAP-38 / SEP-23 L-strkey (liquidity pool). Decodes to the
    // same 32-byte hash the pool CTE matches on, so feeding `hash_bytes`
    // dispatches to the existing pool lookup without an extra SQL branch.
    //
    // Partial L-prefix is intentionally NOT classified: pool storage is
    // raw `BYTEA(32)` with no text mirror column, so a `LIKE 'L%'`
    // prefix scan has nothing to scan. Partial-L prefix matching is
    // tracked in backlog task 0271.
    let upper = q.to_ascii_uppercase();
    if let Ok(stellar_strkey::LiquidityPool(bytes)) =
        stellar_strkey::LiquidityPool::from_string(&upper)
    {
        out.hash_bytes = Some(bytes.to_vec());
        return out;
    }

    // Fully-qualified asset, `CODE:ISSUER` or `CODE-ISSUER`. This is the most
    // precise thing a user can type and it used to classify as nothing at all:
    // the asset arm then hunted a 60+ character needle through ≤12 character
    // codes (provably empty) and the account arm never fired because the string
    // does not start with `G`. The most specific query returned a blank page
    // while the vaguest one returned impostors (task 0485).
    if let Some(pair) = split_code_issuer(q) {
        out.code_issuer = Some(pair);
        return out;
    }

    // StrKey shape (full or prefix of G… / C…). The DB index is
    // `text_pattern_ops` so prefix `LIKE 'PREFIX%'` is the served
    // branch — both the full StrKey and any non-empty prefix work
    // identically against the index. Full vs partial is read off the
    // string length by `Classified::is_fully_typed`.
    if is_strkey_prefix(&upper, 'G') || is_strkey_prefix(&upper, 'C') {
        out.strkey_prefix = Some(upper);
        return out;
    }

    out
}

/// Split a fully-qualified asset into `(code, issuer)`.
///
/// Splits on the LAST `:` or `-`, which is unambiguous: a Stellar asset code is
/// `alphanum4` / `alphanum12`, so neither separator can occur inside one, and a
/// G-StrKey is base32 (`A-Z`, `2-7`) so neither can occur in the issuer either.
/// The issuer is validated in full — `from_string` checks the CRC — so a typo'd
/// key falls through to the ordinary substring search rather than returning a
/// confidently empty page.
fn split_code_issuer(q: &str) -> Option<(String, String)> {
    let (code, issuer) = q.rsplit_once([':', '-'])?;
    if code.is_empty() || code.len() > 12 || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    // Codes are case-sensitive on-chain and matched case-insensitively here, so
    // the original casing is kept; only the StrKey is normalised.
    let issuer = issuer.to_ascii_uppercase();
    stellar_strkey::ed25519::PublicKey::from_string(&issuer).ok()?;
    Some((code.to_string(), issuer))
}

/// Returns true when `s` could be a prefix of a StrKey starting with
/// `prefix`: it begins with `prefix`, every byte is in the StrKey
/// base32 alphabet (`A-Z` and `2-7`), and length ∈ [2, 56].
///
/// Cheap checks (length + prefix byte) come first so the alphabet scan
/// only runs on candidates that already passed the shape gate.
fn is_strkey_prefix(s: &str, prefix: char) -> bool {
    let bytes = s.as_bytes();
    let len = bytes.len();
    if !(2..=56).contains(&len) || bytes[0] != prefix as u8 {
        return false;
    }
    bytes.iter().all(|b| matches!(b, b'A'..=b'Z' | b'2'..=b'7'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_G: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAT";
    const FULL_C: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJ";

    #[test]
    fn classifies_64_hex_as_hash_bytes() {
        let q = "a".repeat(64);
        let out = classify(&q);
        assert_eq!(out.hash_bytes.as_ref().map(Vec::len), Some(32));
        assert!(out.strkey_prefix.is_none());
    }

    #[test]
    fn classifies_full_g_strkey() {
        let out = classify(FULL_G);
        assert_eq!(out.strkey_prefix.as_deref(), Some(FULL_G));
        assert!(out.hash_bytes.is_none());
    }

    #[test]
    fn classifies_full_c_strkey() {
        let out = classify(FULL_C);
        assert_eq!(out.strkey_prefix.as_deref(), Some(FULL_C));
    }

    #[test]
    fn classifies_strkey_prefix_lowercase_input() {
        // Lowercase G prefix should normalise to upper-case; alphabet
        // check happens after normalisation.
        let q = "gaaa";
        let out = classify(q);
        assert_eq!(out.strkey_prefix.as_deref(), Some("GAAA"));
    }

    #[test]
    fn rejects_garbage_text() {
        let out = classify("hello world");
        assert!(out.hash_bytes.is_none());
        assert!(out.strkey_prefix.is_none());
    }

    #[test]
    fn classifies_full_l_strkey_as_pool_hash_bytes() {
        // Full L-strkey decodes to the same 32-byte hash the pool CTE
        // matches on, so it must populate `hash_bytes` (not
        // `strkey_prefix`) so `pool_hits` exact-match fires.
        let strkey = stellar_strkey::LiquidityPool([0u8; 32]).to_string();
        let out = classify(&strkey);
        assert_eq!(out.hash_bytes.as_deref(), Some([0u8; 32].as_slice()));
        assert!(out.strkey_prefix.is_none());
    }

    #[test]
    fn rejects_partial_l_strkey() {
        // Partial L-prefix is not a valid SEP-23 strkey (no CRC payload).
        // Falls through to no classification — broad search has no pool
        // text column to scan against (CH-era follow-up).
        let out = classify("LAB");
        assert!(out.hash_bytes.is_none());
        assert!(out.strkey_prefix.is_none());
    }

    /// A CRC-valid account StrKey. `FULL_G` above is shape-only — the prefix arm
    /// never checks the checksum — but the `CODE:ISSUER` arm does, so it needs a
    /// key the decoder actually accepts.
    fn valid_g() -> String {
        // `format!` (Display), not the inherent `to_string()` — that one returns
        // a `heapless::String`, the same trap `common::strkey` documents.
        format!("{}", stellar_strkey::ed25519::PublicKey([0u8; 32]))
    }

    #[test]
    fn classifies_code_issuer_on_both_separators() {
        // `:` is the canonical SEP / SDK form; `-` is what our own
        // `/assets/:id` routes emit, so users paste it straight back.
        let g = valid_g();
        for q in [format!("USDC:{g}"), format!("USDC-{g}")] {
            let out = classify(&q);
            assert_eq!(
                out.code_issuer,
                Some(("USDC".to_string(), g.clone())),
                "failed for {q:?}"
            );
            // Must not also fire the substring / prefix arms.
            assert!(out.strkey_prefix.is_none());
            assert!(out.hash_bytes.is_none());
        }
    }

    #[test]
    fn code_issuer_keeps_code_case_and_uppercases_the_strkey() {
        let g = valid_g();
        let out = classify(&format!("uSdC:{}", g.to_lowercase()));
        assert_eq!(out.code_issuer, Some(("uSdC".to_string(), g)));
    }

    #[test]
    fn code_issuer_rejects_a_bad_issuer_checksum() {
        // Last character flipped — valid base32, wrong CRC. Falls through to the
        // ordinary search rather than answering with a confident empty page.
        let g = valid_g();
        let flipped = if g.ends_with('A') { 'B' } else { 'A' };
        let bad = format!("{}{flipped}", &g[..g.len() - 1]);
        let out = classify(&format!("USDC:{bad}"));
        assert!(out.code_issuer.is_none());
        assert!(out.strkey_prefix.is_none());
    }

    #[test]
    fn code_issuer_rejects_a_code_longer_than_alphanum12() {
        let out = classify(&format!("THIRTEENCHARS:{}", valid_g()));
        assert!(out.code_issuer.is_none());
    }

    #[test]
    fn code_issuer_rejects_a_non_alphanumeric_code() {
        // A hyphenated word ahead of the key is not an asset code; splitting on
        // the LAST separator keeps this from being read as one.
        let out = classify(&format!("not_a_code:{}", valid_g()));
        assert!(out.code_issuer.is_none());
    }

    #[test]
    fn a_bare_strkey_is_still_a_prefix_not_a_code_issuer() {
        // Regression guard: no separator, so the new arm must not shadow the
        // account/contract prefix classification.
        let g = valid_g();
        let out = classify(&g);
        assert!(out.code_issuer.is_none());
        assert_eq!(out.strkey_prefix.as_deref(), Some(g.as_str()));
    }

    #[test]
    fn short_strkey_prefix_under_two_chars_rejected() {
        // Single-char "G" is too narrow — would force a full-table
        // index range scan. Reject as garbage.
        let out = classify("G");
        assert!(out.strkey_prefix.is_none());
    }
}
