//! Memo extraction from transaction envelopes.

use stellar_xdr::Memo;

/// Extract memo type and value from a Stellar Memo.
///
/// Returns `(memo_type, memo_value)` where both are `None` for `Memo::None`.
/// A `MEMO_TEXT` is 28 arbitrary bytes by protocol; when they are not valid
/// UTF-8 the bytes are kept as hex under the type `text_hex` — never a
/// placeholder string, which would be indistinguishable from a memo that
/// really says that (deep review of task 0540, 2026-09-07).
pub fn extract_memo(memo: &Memo) -> (Option<String>, Option<String>) {
    match memo {
        Memo::None => (None, None),
        Memo::Text(text) => match std::str::from_utf8(text.as_slice()) {
            Ok(s) => (Some("text".to_string()), Some(s.to_string())),
            Err(_) => (
                Some("text_hex".to_string()),
                Some(hex::encode(text.as_slice())),
            ),
        },
        Memo::Id(id) => (Some("id".to_string()), Some(id.to_string())),
        Memo::Hash(hash) => (Some("hash".to_string()), Some(hex::encode(hash.0))),
        Memo::Return(ret) => (Some("return".to_string()), Some(hex::encode(ret.0))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memo_text_valid_utf8() {
        let text = stellar_xdr::StringM::try_from("hello".as_bytes().to_vec()).unwrap();
        let (t, v) = extract_memo(&Memo::Text(text));
        assert_eq!(t.unwrap(), "text");
        assert_eq!(v.unwrap(), "hello");
    }

    #[test]
    fn memo_text_invalid_utf8_is_kept_as_hex_bytes() {
        let invalid = stellar_xdr::StringM::try_from(vec![0xFF, 0xFE]).unwrap();
        let (t, v) = extract_memo(&Memo::Text(invalid));
        assert_eq!(t.unwrap(), "text_hex");
        assert_eq!(v.unwrap(), "fffe");
    }

    #[test]
    fn memo_none() {
        let (t, v) = extract_memo(&Memo::None);
        assert!(t.is_none());
        assert!(v.is_none());
    }

    #[test]
    fn memo_id() {
        let (t, v) = extract_memo(&Memo::Id(12345));
        assert_eq!(t.unwrap(), "id");
        assert_eq!(v.unwrap(), "12345");
    }

    #[test]
    fn memo_hash() {
        let hash = [0xab; 32];
        let (t, v) = extract_memo(&Memo::Hash(stellar_xdr::Hash(hash)));
        assert_eq!(t.unwrap(), "hash");
        assert_eq!(v.unwrap(), "ab".repeat(32));
    }

    #[test]
    fn memo_return() {
        let hash = [0xcd; 32];
        let (t, v) = extract_memo(&Memo::Return(stellar_xdr::Hash(hash)));
        assert_eq!(t.unwrap(), "return");
        assert_eq!(v.unwrap(), "cd".repeat(32));
    }
}
