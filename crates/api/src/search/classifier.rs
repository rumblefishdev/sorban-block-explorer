//! Query classifier: maps raw `q` to the `(hash_bytes, strkey_prefix)`
//! pair consumed by `22_get_search.sql`.
//!
//! Two derived inputs only — the SQL itself decides which CTE branches
//! fire based on which input is non-NULL. Keeping the classifier this
//! narrow means there is no per-entity dispatch logic in Rust to drift
//! from the SQL contract.

use crate::common::strkey::is_strkey_shape;

/// Classifier output. `None` means "this branch should not fire".
#[derive(Debug, Default, Clone)]
pub struct Classified {
    /// 32 bytes if `q` parses as 32-byte hex or base64; drives the
    /// `transaction` and `pool` exact-match CTEs.
    pub hash_bytes: Option<Vec<u8>>,
    /// Upper-cased StrKey or its prefix when `q` matches Stellar StrKey
    /// shape (full 56 chars or any prefix of `G…` / `C…`); drives the
    /// `account` and `contract` prefix CTEs.
    pub strkey_prefix: Option<String>,
    /// True when `q` is a fully-typed entity id that should redirect
    /// at the route level (no broad search) when an entity exists:
    /// 64-hex-char (32-byte) hash, full 56-char `G…` StrKey, or full
    /// 56-char `C…` StrKey.
    pub is_fully_typed: bool,
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
        out.is_fully_typed = true;
        return out;
    }

    // 32-byte base64. Stellar tooling sometimes hands hashes around as
    // base64-encoded 32-byte payloads; the SQL accepts BYTEA(32) from
    // either source so we normalise here.
    //
    // Standard alphabet only (`+/`); URL-safe is intentionally not
    // accepted — Stellar tools emit standard base64 and accepting
    // both opens classifier ambiguity for short strings.
    if let Some(bytes) = decode_base64_32(q) {
        out.hash_bytes = Some(bytes.to_vec());
        out.is_fully_typed = true;
        return out;
    }

    // StrKey shape (full or prefix of G… / C…). The DB index is
    // `text_pattern_ops` so prefix `LIKE 'PREFIX%'` is the served
    // branch — both the full StrKey and any non-empty prefix work
    // identically against the index.
    let upper = q.to_ascii_uppercase();
    if is_strkey_prefix(&upper, 'G') || is_strkey_prefix(&upper, 'C') {
        out.strkey_prefix = Some(upper.clone());
        out.is_fully_typed = is_strkey_shape(&upper, 'G') || is_strkey_shape(&upper, 'C');
        return out;
    }

    out
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

/// Try to decode `s` as standard-alphabet base64 representing exactly
/// 32 bytes. Length 44 with `=` padding is the canonical encoding; we
/// tolerate length 43 (no padding) for callers that strip it. Returns
/// `None` for any other length / charset / payload size.
fn decode_base64_32(s: &str) -> Option<[u8; 32]> {
    use base64::Engine;
    if !matches!(s.len(), 43 | 44) {
        return None;
    }
    // Reject anything that's clearly out of the standard alphabet to
    // avoid `decode` accepting whitespace or URL-safe chars.
    let trimmed = s.trim_end_matches('=');
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    {
        return None;
    }
    // STANDARD engine refuses unpadded input. Re-pad to 44 chars when
    // the caller stripped the trailing `=`, then run a single decode
    // path. Avoids carrying two engine variants for what is the same
    // 32-byte payload either way.
    let padded: String;
    let to_decode: &str = if s.len() == 43 {
        padded = format!("{s}=");
        &padded
    } else {
        s
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(to_decode)
        .ok()?;
    bytes.try_into().ok()
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
        assert!(out.is_fully_typed);
        assert!(out.strkey_prefix.is_none());
    }

    #[test]
    fn classifies_full_g_strkey() {
        let out = classify(FULL_G);
        assert_eq!(out.strkey_prefix.as_deref(), Some(FULL_G));
        assert!(out.is_fully_typed);
        assert!(out.hash_bytes.is_none());
    }

    #[test]
    fn classifies_full_c_strkey() {
        let out = classify(FULL_C);
        assert_eq!(out.strkey_prefix.as_deref(), Some(FULL_C));
        assert!(out.is_fully_typed);
    }

    #[test]
    fn classifies_strkey_prefix_lowercase_input() {
        // Lowercase G prefix should normalise to upper-case; alphabet
        // check happens after normalisation.
        let q = "gaaa";
        let out = classify(q);
        assert_eq!(out.strkey_prefix.as_deref(), Some("GAAA"));
        assert!(!out.is_fully_typed);
    }

    #[test]
    fn rejects_garbage_text() {
        let out = classify("hello world");
        assert!(out.hash_bytes.is_none());
        assert!(out.strkey_prefix.is_none());
        assert!(!out.is_fully_typed);
    }

    #[test]
    fn classifies_base64_32_bytes() {
        // 32-byte payload encoded standard base64 — 44 chars with `=` padding.
        let raw = [0x42u8; 32];
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
        assert_eq!(encoded.len(), 44);
        let out = classify(&encoded);
        assert_eq!(out.hash_bytes.as_deref(), Some(raw.as_slice()));
        assert!(out.is_fully_typed);
    }

    #[test]
    fn classifies_base64_32_bytes_unpadded() {
        // Same 32-byte payload but caller stripped the trailing `=`.
        // 44-char padded → 43-char unpadded. The decoder MUST tolerate
        // the unpadded form because some Stellar tools emit it that way.
        let raw = [0x42u8; 32];
        use base64::Engine;
        let padded = base64::engine::general_purpose::STANDARD.encode(raw);
        let unpadded = padded.trim_end_matches('=').to_string();
        assert_eq!(unpadded.len(), 43);
        let out = classify(&unpadded);
        assert_eq!(out.hash_bytes.as_deref(), Some(raw.as_slice()));
        assert!(out.is_fully_typed);
    }

    #[test]
    fn short_strkey_prefix_under_two_chars_rejected() {
        // Single-char "G" is too narrow — would force a full-table
        // index range scan. Reject as garbage.
        let out = classify("G");
        assert!(out.strkey_prefix.is_none());
    }
}
