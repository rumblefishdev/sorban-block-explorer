//! NFT event detection from extracted Soroban events.
//!
//! Identifies mint, transfer, and burn events that match NFT patterns
//! (SEP-0050 and known non-standard conventions). Produces `NftEvent`
//! structs for consumption by task 0027 (NFT state derivation).
//!
//! Detection is based on event topic patterns. The first topic is expected
//! to be a Symbol naming the event kind. Case-insensitive matching handles
//! both standard ("transfer") and non-standard ("Transfer") conventions.

use serde_json::Value;

use crate::types::{ExtractedEvent, NftEvent};

/// Detect NFT-related events from a list of extracted events.
///
/// Scans for topic patterns matching NFT mint/transfer/burn events.
/// Only events with a non-null `contract_id` are considered.
///
/// Returns detected `NftEvent` items. Events that don't match any
/// NFT pattern are silently skipped.
pub fn detect_nft_events(events: &[ExtractedEvent]) -> Vec<NftEvent> {
    let mut nft_events = Vec::new();

    for event in events {
        if event.event_type != "contract" {
            continue;
        }
        let Some(ref contract_id) = event.contract_id else {
            continue;
        };

        let topics = match event.topics.as_array() {
            Some(t) if !t.is_empty() => t,
            _ => continue,
        };

        let first_topic = topic_symbol_value(&topics[0]);
        let first_lower = first_topic.to_ascii_lowercase();

        match first_lower.as_str() {
            "transfer" => {
                if let Some(nft) = try_parse_transfer(contract_id, &topics[1..], &event.data, event)
                {
                    nft_events.push(nft);
                }
            }
            "mint" => {
                if let Some(nft) = try_parse_mint(contract_id, &topics[1..], &event.data, event) {
                    nft_events.push(nft);
                }
            }
            "burn" => {
                if let Some(nft) = try_parse_burn(contract_id, &topics[1..], &event.data, event) {
                    nft_events.push(nft);
                }
            }
            _ => {}
        }
    }

    nft_events
}

/// Try to parse a transfer event as an NFT transfer.
///
/// SEP-0050 pattern: topics = [Symbol("transfer"), Address(from), Address(to)], data = token_id
/// We only emit if the data looks like a token ID (not an i128 amount).
fn try_parse_transfer(
    contract_id: &str,
    remaining_topics: &[Value],
    data: &Value,
    event: &ExtractedEvent,
) -> Option<NftEvent> {
    if remaining_topics.len() < 2 {
        return None;
    }

    // Data must look like a token_id, not an i128 amount (fungible token pattern)
    if !looks_like_token_id(data) {
        return None;
    }

    let from = topic_address_value(&remaining_topics[0])?;
    let to = topic_address_value(&remaining_topics[1])?;

    Some(NftEvent {
        transaction_hash: event.transaction_hash.clone(),
        contract_id: contract_id.to_string(),
        event_kind: "transfer".into(),
        token_id: data.clone(),
        from: Some(from),
        to: Some(to),
        ledger_sequence: event.ledger_sequence,
        created_at: event.created_at,
    })
}

/// Try to parse a mint event as an NFT mint.
///
/// SEP-0050 pattern: topics = [Symbol("mint"), Address(to)], data = token_id
fn try_parse_mint(
    contract_id: &str,
    remaining_topics: &[Value],
    data: &Value,
    event: &ExtractedEvent,
) -> Option<NftEvent> {
    if remaining_topics.is_empty() {
        return None;
    }

    if !looks_like_token_id(data) {
        return None;
    }

    let to = topic_address_value(&remaining_topics[0])?;

    Some(NftEvent {
        transaction_hash: event.transaction_hash.clone(),
        contract_id: contract_id.to_string(),
        event_kind: "mint".into(),
        token_id: data.clone(),
        from: None,
        to: Some(to),
        ledger_sequence: event.ledger_sequence,
        created_at: event.created_at,
    })
}

/// Try to parse a burn event as an NFT burn.
///
/// Burn is not standardized in SEP-0050 core but some contracts emit it.
/// Pattern: topics = [Symbol("burn"), Address(from)], data = token_id
fn try_parse_burn(
    contract_id: &str,
    remaining_topics: &[Value],
    data: &Value,
    event: &ExtractedEvent,
) -> Option<NftEvent> {
    if remaining_topics.is_empty() {
        return None;
    }

    if !looks_like_token_id(data) {
        return None;
    }

    let from = topic_address_value(&remaining_topics[0])?;

    Some(NftEvent {
        transaction_hash: event.transaction_hash.clone(),
        contract_id: contract_id.to_string(),
        event_kind: "burn".into(),
        token_id: data.clone(),
        from: Some(from),
        to: None,
        ledger_sequence: event.ledger_sequence,
        created_at: event.created_at,
    })
}

/// Check if data looks like a token ID (scalar value, not a complex structure).
///
/// Both SEP-0041 (fungible) and SEP-0050 (NFT) use the same topic pattern
/// for transfer/mint events. The data type alone cannot reliably distinguish
/// them — some NFT contracts (e.g. jamesbachini) use i128 for token IDs.
///
/// This check only rejects clearly non-token data (void, maps, vecs).
/// Definitive NFT vs fungible classification should use WASM spec analysis
/// from `contract.rs` (task 0027 responsibility).
fn looks_like_token_id(data: &Value) -> bool {
    let type_str = data.get("type").and_then(|v| v.as_str()).unwrap_or("");
    !matches!(type_str, "void" | "map" | "vec" | "error")
}

/// Extract a symbol string from a tagged ScVal JSON topic.
///
/// Only matches topics with `"type": "sym"` to avoid false positives
/// from other topic types that happen to have string values.
fn topic_symbol_value(topic: &Value) -> String {
    let type_str = topic.get("type").and_then(|v| v.as_str());
    if type_str == Some("sym")
        && let Some(s) = topic.get("value").and_then(|v| v.as_str())
    {
        return s.to_string();
    }
    String::new()
}

/// Extract an address string from a tagged ScVal JSON topic.
///
/// Only accepts topics typed as "address" with a non-empty string value.
/// Returns `None` for non-address topics so callers can skip events
/// with invalid address fields.
fn topic_address_value(topic: &Value) -> Option<String> {
    let type_str = topic.get("type").and_then(|v| v.as_str());
    if type_str == Some("address")
        && let Some(s) = topic.get("value").and_then(|v| v.as_str())
        && !s.is_empty()
    {
        return Some(s.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_event(contract_id: &str, topics: Vec<Value>, data: Value) -> ExtractedEvent {
        ExtractedEvent {
            transaction_hash: "abcd1234".into(),
            event_type: "contract".into(),
            contract_id: Some(contract_id.into()),
            topics: json!(topics),
            data,
            event_index: 0,
            ledger_sequence: 100,
            created_at: 1700000000,
        }
    }

    #[test]
    fn detect_nft_transfer() {
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "transfer"}),
                json!({"type": "address", "value": "GFROM..."}),
                json!({"type": "address", "value": "GTO..."}),
            ],
            json!({"type": "u32", "value": 42}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "transfer");
        assert_eq!(nft_events[0].from.as_deref(), Some("GFROM..."));
        assert_eq!(nft_events[0].to.as_deref(), Some("GTO..."));
        assert_eq!(nft_events[0].token_id["value"], 42);
    }

    #[test]
    fn detect_nft_transfer_case_insensitive() {
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "Transfer"}),
                json!({"type": "address", "value": "GFROM..."}),
                json!({"type": "address", "value": "GTO..."}),
            ],
            json!({"type": "u32", "value": 1}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "transfer");
    }

    #[test]
    fn i128_token_id_not_excluded() {
        // jamesbachini NFT contract uses i128 for token IDs
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "transfer"}),
                json!({"type": "address", "value": "GFROM..."}),
                json!({"type": "address", "value": "GTO..."}),
            ],
            json!({"type": "i128", "value": "5"}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "transfer");
    }

    #[test]
    fn skip_void_data() {
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "transfer"}),
                json!({"type": "address", "value": "GFROM..."}),
                json!({"type": "address", "value": "GTO..."}),
            ],
            json!({"type": "void", "value": null}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert!(nft_events.is_empty());
    }

    #[test]
    fn detect_nft_mint() {
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "mint"}),
                json!({"type": "address", "value": "GTO..."}),
            ],
            json!({"type": "u32", "value": 1}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "mint");
        assert!(nft_events[0].from.is_none());
        assert_eq!(nft_events[0].to.as_deref(), Some("GTO..."));
    }

    #[test]
    fn detect_nft_burn() {
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "burn"}),
                json!({"type": "address", "value": "GFROM..."}),
            ],
            json!({"type": "u32", "value": 5}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "burn");
        assert_eq!(nft_events[0].from.as_deref(), Some("GFROM..."));
        assert!(nft_events[0].to.is_none());
    }

    #[test]
    fn skip_system_events() {
        let mut event = make_event(
            "CABC123",
            vec![json!({"type": "sym", "value": "transfer"})],
            json!({"type": "u32", "value": 1}),
        );
        event.event_type = "system".into();

        let nft_events = detect_nft_events(&[event]);
        assert!(nft_events.is_empty());
    }

    #[test]
    fn skip_events_without_contract_id() {
        let mut event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "mint"}),
                json!({"type": "address", "value": "GTO..."}),
            ],
            json!({"type": "u32", "value": 1}),
        );
        event.contract_id = None;

        let nft_events = detect_nft_events(&[event]);
        assert!(nft_events.is_empty());
    }

    #[test]
    fn skip_unrecognized_event_topics() {
        let event = make_event(
            "CABC123",
            vec![json!({"type": "sym", "value": "approve"})],
            json!({"type": "u32", "value": 1}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert!(nft_events.is_empty());
    }

    #[test]
    fn transfer_needs_two_address_topics() {
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "transfer"}),
                // Only one address — not enough
                json!({"type": "address", "value": "GFROM..."}),
            ],
            json!({"type": "u32", "value": 1}),
        );

        let nft_events = detect_nft_events(&[event]);
        // Needs from + to
        assert!(nft_events.is_empty());
    }
}
