//! SEP-0041 transfer-event classification for Soroban contract events.
//!
//! The transfer topic shape is the stable SEP-0041 convention:
//! `topics == [Symbol("transfer"), Address(from), Address(to), ...]`
//! with the amount carried in the event data payload. NFT transfers reuse
//! the topic shape but carry a `token_id` in `data`; we surface those as
//! transfers with `amount = None` so fungible vs. NFT is unambiguous at
//! the call site.
//!
//! Consumed at ingest (indexer staging — participant registration) and
//! at read time (API E10 — filter token transactions to transfer-emitting
//! ones). Centralising the shape rules here avoids drift between the two
//! paths.

use serde_json::Value;

/// A decoded SEP-0041 transfer event.
///
/// `amount` is `None` when `data` does not carry a numeric ScVal — the
/// canonical case is an NFT transfer whose `data` is a `token_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    pub from: String,
    pub to: String,
    pub amount: Option<String>,
}

/// Shape-only predicate: returns `true` iff `topics` matches the
/// SEP-0041 transfer layout. Does not inspect the event data.
pub fn is_transfer_event(topics: &Value) -> bool {
    parse_transfer_shape(topics).is_some()
}

/// Decode only the `(from, to)` participants from a transfer event's
/// topics. Returns `None` when topics do not match the SEP-0041 shape.
/// Cheaper than `parse_transfer` when the amount is not needed (e.g.
/// indexer participant registration).
pub fn transfer_participants(topics: &Value) -> Option<(String, String)> {
    parse_transfer_shape(topics)
}

/// Decode a transfer event's `(from, to, amount)` from its topics + data.
/// Returns `None` when topics do not match the SEP-0041 shape.
pub fn parse_transfer(topics: &Value, data: &Value) -> Option<Transfer> {
    let (from, to) = parse_transfer_shape(topics)?;
    Some(Transfer {
        from,
        to,
        amount: numeric_scval(data),
    })
}

fn parse_transfer_shape(topics: &Value) -> Option<(String, String)> {
    let arr = topics.as_array()?;
    if arr.len() < 3 {
        return None;
    }
    if arr[0].get("type").and_then(Value::as_str)? != "sym" {
        return None;
    }
    let sym = arr[0].get("value").and_then(Value::as_str)?;
    if !sym.eq_ignore_ascii_case("transfer") {
        return None;
    }
    let from = address_topic(&arr[1])?;
    let to = address_topic(&arr[2])?;
    Some((from, to))
}

fn address_topic(topic: &Value) -> Option<String> {
    if topic.get("type").and_then(Value::as_str)? != "address" {
        return None;
    }
    let s = topic.get("value").and_then(Value::as_str)?;
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Extract a NUMERIC(39,0)-compatible decimal string from a ScVal-typed JSON
/// value. Accepts signed/unsigned 32/64 bit ints directly, and 128/256 bit
/// ints which stellar-xdr encodes as JSON strings.
fn numeric_scval(data: &Value) -> Option<String> {
    let ty = data.get("type").and_then(Value::as_str)?;
    let val = data.get("value")?;
    match ty {
        "u32" | "i32" | "u64" | "i64" => val
            .as_i64()
            .map(|n| n.to_string())
            .or_else(|| val.as_u64().map(|n| n.to_string())),
        "u128" | "i128" | "u256" | "i256" => val.as_str().map(str::to_string),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sym(value: &str) -> Value {
        json!({ "type": "sym", "value": value })
    }

    fn addr(value: &str) -> Value {
        json!({ "type": "address", "value": value })
    }

    fn transfer_topics(from: &str, to: &str) -> Value {
        json!([sym("transfer"), addr(from), addr(to)])
    }

    #[test]
    fn is_transfer_accepts_canonical_three_topic_shape() {
        let topics = transfer_topics("GA...FROM", "GA...TO");
        assert!(is_transfer_event(&topics));
    }

    #[test]
    fn is_transfer_accepts_four_topic_shape_with_asset_code() {
        let topics = json!([sym("transfer"), addr("GA"), addr("GB"), sym("USDC")]);
        assert!(is_transfer_event(&topics));
    }

    #[test]
    fn is_transfer_is_case_insensitive_on_symbol() {
        assert!(is_transfer_event(&json!([
            sym("Transfer"),
            addr("GA"),
            addr("GB")
        ])));
        assert!(is_transfer_event(&json!([
            sym("TRANSFER"),
            addr("GA"),
            addr("GB")
        ])));
    }

    #[test]
    fn is_transfer_rejects_too_few_topics() {
        assert!(!is_transfer_event(&json!([sym("transfer"), addr("GA")])));
    }

    #[test]
    fn is_transfer_rejects_non_symbol_prefix() {
        assert!(!is_transfer_event(&json!([
            addr("GA"),
            addr("GB"),
            addr("GC")
        ])));
    }

    #[test]
    fn is_transfer_rejects_other_symbol() {
        assert!(!is_transfer_event(&json!([
            sym("mint"),
            addr("GA"),
            addr("GB")
        ])));
    }

    #[test]
    fn is_transfer_rejects_non_address_participants() {
        assert!(!is_transfer_event(&json!([
            sym("transfer"),
            sym("GA"),
            addr("GB")
        ])));
        assert!(!is_transfer_event(&json!([
            sym("transfer"),
            addr("GA"),
            sym("GB")
        ])));
    }

    #[test]
    fn is_transfer_rejects_empty_address_value() {
        assert!(!is_transfer_event(&json!([
            sym("transfer"),
            addr(""),
            addr("GB")
        ])));
    }

    #[test]
    fn parse_transfer_extracts_u64_amount() {
        let t = parse_transfer(
            &transfer_topics("GA", "GB"),
            &json!({ "type": "u64", "value": 123_456_u64 }),
        )
        .unwrap();
        assert_eq!(t.from, "GA");
        assert_eq!(t.to, "GB");
        assert_eq!(t.amount.as_deref(), Some("123456"));
    }

    #[test]
    fn parse_transfer_extracts_i128_amount_from_string() {
        let t = parse_transfer(
            &transfer_topics("GA", "GB"),
            &json!({ "type": "i128", "value": "170141183460469231731687303715884105727" }),
        )
        .unwrap();
        assert_eq!(
            t.amount.as_deref(),
            Some("170141183460469231731687303715884105727")
        );
    }

    #[test]
    fn parse_transfer_extracts_u256_amount_from_string() {
        let t = parse_transfer(
            &transfer_topics("GA", "GB"),
            &json!({ "type": "u256", "value": "999999999999999999999999" }),
        )
        .unwrap();
        assert_eq!(t.amount.as_deref(), Some("999999999999999999999999"));
    }

    #[test]
    fn parse_transfer_returns_none_amount_for_nft_token_id() {
        // NFT-style transfer: data carries a token_id (non-numeric ScVal).
        let t = parse_transfer(
            &transfer_topics("GA", "GB"),
            &json!({ "type": "bytes", "value": "abcd" }),
        )
        .unwrap();
        assert_eq!(t.from, "GA");
        assert_eq!(t.to, "GB");
        assert!(t.amount.is_none());
    }

    #[test]
    fn parse_transfer_returns_none_for_non_transfer_topic() {
        let out = parse_transfer(
            &json!([sym("mint"), addr("GA"), addr("GB")]),
            &json!({ "type": "u64", "value": 1 }),
        );
        assert!(out.is_none());
    }
}
