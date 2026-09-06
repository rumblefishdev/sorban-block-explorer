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
use tracing::warn;

use crate::sac::{SacOverride, sac_override_from_event_topics, topic_symbol_value};
use crate::scval::map_get;
use crate::types::{EventSource, ExtractedEvent, NftEvent};
use domain::ContractEventType;

/// Detect NFT-related events from a list of extracted events.
///
/// Scans for topic patterns matching NFT mint/transfer/burn events.
/// Only events with a non-null `contract_id` are considered.
///
/// Returns detected `NftEvent` items. Events that don't match any
/// NFT pattern are silently skipped.
///
/// `net_id` (`network_id(passphrase)`) gates out classic-asset SAC events at
/// their source (task 0294): a CAP-67 SAC `transfer`/`mint`/`burn` carries the
/// SEP-11 asset string in its LAST topic and an i128 AMOUNT in `data`, so when
/// `derive_sac(asset) == emitter` it is provably a fungible amount and can never
/// be an NFT. Dropping it here stops the amount being mis-read as a token_id and
/// minted as a false NFT candidate. The gate is false-negative-only — a bespoke
/// contract (`derived != emitter`) or non-SAC signature returns `None`, so real
/// NFTs are unaffected.
pub fn detect_nft_events(events: &[ExtractedEvent], net_id: &[u8; 32]) -> Vec<NftEvent> {
    let mut nft_events = Vec::new();

    for event in events {
        if event.event_type != ContractEventType::Contract {
            continue;
        }
        // Skip diagnostic-container Contract-typed copies — when
        // diagnostic mode is enabled, every per-op consensus Contract
        // event is also present byte-identically in `v4.diagnostic_events`;
        // without this guard NFT detection would double-emit
        // transfer/mint/burn rows (task 0182).
        if event.source == EventSource::Diagnostic {
            continue;
        }
        let Some(ref contract_id) = event.contract_id else {
            continue;
        };

        let topics = match event.topics.as_array() {
            Some(t) if !t.is_empty() => t,
            _ => continue,
        };

        // Task 0294 — drop crypto-proven classic-asset SAC events before they
        // can be minted as false NFT candidates. `derive_sac(asset) == emitter`
        // is cryptographic proof the i128 in `data` is a transfer AMOUNT, not an
        // NFT token_id. `None` for bespoke contracts / non-SAC signatures, so
        // real NFTs pass through (false-negative-only).
        if sac_override_from_event_topics(contract_id, &event.topics, net_id).is_some() {
            continue;
        }

        let first_topic = topic_symbol_value(&topics[0]);
        let first_lower = first_topic.to_ascii_lowercase();

        // consecutive_mint (OZ Consecutive / EIP-2309) expands a [from,to] range
        // to N mints, so it yields multiple events — handle it before the
        // single-event symbols below.
        if first_lower == "consecutive_mint" {
            match try_parse_consecutive_mint(contract_id, &topics[1..], &event.data, event) {
                Some(mut evs) => nft_events.append(&mut evs),
                None => maybe_tripwire(contract_id, &first_lower, event, topics.len()),
            }
            continue;
        }

        let parsed = match first_lower.as_str() {
            "transfer" => try_parse_transfer(contract_id, &topics[1..], &event.data, event),
            "mint" => try_parse_mint(contract_id, &topics[1..], &event.data, event),
            "burn" => try_parse_burn(contract_id, &topics[1..], &event.data, event),
            // Not an NFT-candidate symbol — skip silently (no tripwire).
            _ => continue,
        };

        match parsed {
            Some(nft) => nft_events.push(nft),
            // Candidate symbol but no known arg shape (Shape A topics+scalar,
            // Shape B packed-vec, Shape C topics+map{token_id}) parsed — drop and
            // tripwire so genuinely-unhandled NFT shapes surface. Fungible maps
            // are suppressed inside maybe_tripwire.
            None => maybe_tripwire(contract_id, &first_lower, event, topics.len()),
        }
    }

    nft_events
}

/// Crypto-proven un-deployed-SAC overrides for the ledger's events (task 0323).
///
/// An un-deployed SAC emits a CAP-67 unified event under its reserved C… address
/// without ever being deployed. [`sac_override_from_event_topics`] proves
/// `emitter == derive_sac(asset)`. We collect those overrides (deduped by contract
/// id) so staging models them as ASSETS, not `soroban_contracts` rows: the override
/// id suppresses the Pass-2 FK stub, and its `identity` seeds the `assets` row.
/// Replaces the trustline-sourced `derive_sac_overrides_from_assets` — an
/// un-deployed SAC matters once it has activity (an event), not at the trustline.
//
// ponytail: re-runs the SAC gate already evaluated in `detect_nft_events`; the
// expensive SHA256 only fires for genuine SAC-shaped events (last topic parses as
// a SEP-11 asset), so the double-cost is bounded. Fold the collect into
// `detect_nft_events` if profiling shows it hot.
pub fn detect_undeployed_sac_overrides(
    events: &[(String, Vec<ExtractedEvent>)],
    net_id: &[u8; 32],
) -> Vec<SacOverride> {
    let mut by_cid: std::collections::HashMap<String, SacOverride> =
        std::collections::HashMap::new();
    for (_tx_hash, evs) in events {
        for ev in evs {
            if ev.source == EventSource::Diagnostic {
                continue;
            }
            let Some(cid) = &ev.contract_id else {
                continue;
            };
            if by_cid.contains_key(cid) {
                continue;
            }
            if let Some(ov) = sac_override_from_event_topics(cid, &ev.topics, net_id) {
                by_cid.insert(cid.clone(), ov);
            }
        }
    }
    by_cid.into_values().collect()
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
    let (addrs, token_id) = extract_args(remaining_topics, data, 2)?;
    Some(NftEvent {
        transaction_hash: event.transaction_hash.clone(),
        contract_id: contract_id.to_string(),
        event_kind: "transfer".into(),
        token_id,
        from: Some(addrs[0].clone()),
        to: Some(addrs[1].clone()),
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
    let (addrs, token_id) = extract_args(remaining_topics, data, 1)?;
    Some(NftEvent {
        transaction_hash: event.transaction_hash.clone(),
        contract_id: contract_id.to_string(),
        event_kind: "mint".into(),
        token_id,
        from: None,
        to: Some(addrs[0].clone()),
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
    let (addrs, token_id) = extract_args(remaining_topics, data, 1)?;
    Some(NftEvent {
        transaction_hash: event.transaction_hash.clone(),
        contract_id: contract_id.to_string(),
        event_kind: "burn".into(),
        token_id,
        from: Some(addrs[0].clone()),
        to: None,
        ledger_sequence: event.ledger_sequence,
        created_at: event.created_at,
    })
}

/// Maximum tokens a single `consecutive_mint` may expand to — guards against a
/// malformed/hostile range exploding memory/CPU in the (Lambda) indexer. Real
/// ranges are tiny (≤146 observed); over-cap ranges are dropped + tripwired, not
/// expanded. Sized to the downstream i16 event-order cap.
const MAX_CONSECUTIVE_RANGE: u64 = 65_535;

/// Parse an OZ Consecutive `consecutive_mint` (EIP-2309 analog) into one mint per
/// token_id in the inclusive `[from_token_id, to_token_id]` range.
///
/// Topics = [Symbol("consecutive_mint"), Address(to)]; data is either
/// `map{from_token_id, to_token_id}` or `vec[from, to]` — both observed on
/// mainnet (2026-06-17) — each an unsigned int. Returns `None` (→ tripwire) for
/// an unrecognised layout, an inverted range, or an implausibly large range.
fn try_parse_consecutive_mint(
    contract_id: &str,
    remaining_topics: &[Value],
    data: &Value,
    event: &ExtractedEvent,
) -> Option<Vec<NftEvent>> {
    let to = remaining_topics.first().and_then(topic_address_value)?;

    let (from_v, to_v) = if let Some(elems) = data_vec_elements(data) {
        (elems.first()?, elems.get(1)?)
    } else {
        (
            map_get(data, "from_token_id")?,
            map_get(data, "to_token_id")?,
        )
    };
    let (from_id, to_id) = consecutive_range(from_v, to_v)?;
    if to_id < from_id || to_id - from_id >= MAX_CONSECUTIVE_RANGE {
        return None;
    }

    Some(
        (from_id..=to_id)
            .map(|id| NftEvent {
                transaction_hash: event.transaction_hash.clone(),
                contract_id: contract_id.to_string(),
                event_kind: "mint".into(),
                token_id: serde_json::json!({ "type": "u64", "value": id }),
                from: None,
                to: Some(to.clone()),
                ledger_sequence: event.ledger_sequence,
                created_at: event.created_at,
            })
            .collect(),
    )
}

/// Extract the `(from, to)` token-id bounds from a `consecutive_mint`'s two range
/// operands, accepting only unsigned-int ScVals (u32/u64; wider ids tripwire).
fn consecutive_range(from_v: &Value, to_v: &Value) -> Option<(u64, u64)> {
    let from_ty = from_v.get("type").and_then(|v| v.as_str())?;
    let to_ty = to_v.get("type").and_then(|v| v.as_str())?;
    if !matches!(from_ty, "u32" | "u64") || !matches!(to_ty, "u32" | "u64") {
        return None;
    }
    let from = from_v.get("value").and_then(|v| v.as_u64())?;
    let to = to_v.get("value").and_then(|v| v.as_u64())?;
    Some((from, to))
}

/// Emit the NFT tripwire unless the unparsed payload is a recognisably-fungible
/// map (CAP-67/SAC `map{amount,to_muxed_id}`) — those are not NFTs and would
/// bury the real signal under ~148M events.
fn maybe_tripwire(contract_id: &str, event_kind: &str, event: &ExtractedEvent, topic_count: usize) {
    if is_fungible_map(&event.data) {
        return;
    }
    warn!(
        target: "xdr_parser::nft",
        contract_id = %contract_id,
        event_kind,
        data_type = event.data.get("type").and_then(|v| v.as_str()).unwrap_or("?"),
        topic_count,
        "NFT event symbol matched but no known arg shape parsed — row dropped. \
         Extend nft.rs shapes if this is a real NFT."
    );
}

/// Check if data looks like a token ID (scalar value, not a complex structure).
///
/// Both SEP-0041 (fungible) and SEP-0050 (NFT) use the same `["transfer",
/// Address(from), Address(to)]` topic pattern for transfer/mint/burn events
/// — only the data payload differentiates them. The payload type alone
/// **cannot** reliably tell NFT from fungible: real mainnet NFTs (e.g. the
/// James Bachini SEP-39 collection `CDA5FGE4LZP4S45LP6AJLWMLKWHVWMKFSIKVYEBSIYOB25NWLKCLL7RY`
/// — confirmed live 2026-05-13 via `stellar contract fetch`) use `i128`
/// for `token_id` (SEP-39 / ERC-721 style), and SEP-41 fungible transfers
/// use `i128` for `amount`. **Both shapes are observed on mainnet.**
///
/// Task 0118 originally tried a Patch C whitelist (rejecting `i128`/`u128`
/// at the parser) but that approach silently drops legitimate SEP-39 NFTs
/// — the 2026-05-12 CH pilot audit sample didn't contain one so the
/// false-negative wasn't visible in measurement; the 2026-05-13 pre-audit
/// re-test against live mainnet RPC found one. **Patch C was reverted.**
///
/// Authoritative NFT-vs-fungible discrimination lives downstream, at
/// persist time, via the WASM-spec-based classifier
/// (`xdr_parser::classify_contract_from_wasm_spec` + the persist routing
/// in `crates/indexer/src/handler/persist/write.rs::resolve_nft_filter`):
///
/// - `Fungible` / `Token` (SAC) verdict → row dropped before INSERT.
/// - `Nft` verdict → row to hot `nfts` table.
/// - `Other` / NULL verdict → row to `nfts_pending` quarantine
///   (task 0217), promoted on later WASM observation.
///
/// This function therefore only rejects clearly non-token shapes (void,
/// maps, vecs, errors). Everything else is forwarded for the classifier
/// to judge.
fn looks_like_token_id(data: &Value) -> bool {
    let type_str = data.get("type").and_then(|v| v.as_str()).unwrap_or("");
    !matches!(type_str, "void" | "map" | "vec" | "error")
}

/// True for a data map recognisable as a fungible/SAC event payload: no
/// `token_id` key, but carrying `amount` or `to_muxed_id` (the CAP-67 muxed
/// transfer/mint shape — ~148M on mainnet vs ~890 NFT `map{token_id}` mints).
/// Used to suppress the NFT tripwire for these known non-NFT maps.
fn is_fungible_map(data: &Value) -> bool {
    map_get(data, "token_id").is_none()
        && (map_get(data, "amount").is_some() || map_get(data, "to_muxed_id").is_some())
}

/// Return the elements of a `Vec`-typed ScVal JSON payload, iff `data` is a
/// `{"type":"vec","value":[…]}`. Used for the "packed" NFT event shape where
/// the event args ride in the data tuple instead of the topics.
fn data_vec_elements(data: &Value) -> Option<&[Value]> {
    if data.get("type").and_then(|v| v.as_str()) != Some("vec") {
        return None;
    }
    data.get("value")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
}

/// Collect every element as an address string, or `None` if any element is
/// not a valid address.
fn collect_addresses(items: &[Value]) -> Option<Vec<String>> {
    items.iter().map(topic_address_value).collect()
}

/// Extract `n_addrs` address strings plus a `token_id` value from an NFT
/// event, tolerating the layouts observed on mainnet:
///
/// - **Shape A** (SEP-50 NFT / SEP-41 standard): addresses in topics, token_id
///   in data.
/// - **Shape B** (packed, non-standard): the whole `(addr…, token_id)` tuple in
///   a data `Vec` — e.g. Bachini / ERC-721-port contracts (verified on-chain).
///
/// Returns `None` for map-shaped or otherwise unrecognised payloads — the
/// caller surfaces those via the `detect_nft_events` tripwire rather than
/// silently dropping them. (A token_id-as-extra-topic layout was considered
/// but cut: no SEP defines it and no on-chain instance was found; the tripwire
/// will surface it if it ever appears.)
fn extract_args(
    remaining_topics: &[Value],
    data: &Value,
    n_addrs: usize,
) -> Option<(Vec<String>, Value)> {
    // Shape A: addresses in topics, token_id scalar in data.
    if remaining_topics.len() >= n_addrs
        && looks_like_token_id(data)
        && let Some(addrs) = collect_addresses(&remaining_topics[..n_addrs])
    {
        return Some((addrs, data.clone()));
    }

    // Shape B: (addresses…, token_id) packed in a data Vec.
    if let Some(elems) = data_vec_elements(data)
        && elems.len() == n_addrs + 1
    {
        let token_id = &elems[n_addrs];
        if looks_like_token_id(token_id)
            && let Some(addrs) = collect_addresses(&elems[..n_addrs])
        {
            return Some((addrs, token_id.clone()));
        }
    }

    // Shape C (canonical OZ / SEP-50): addresses in topics, token_id in a data
    // map under the "token_id" key — the soroban-sdk `#[contractevent]`
    // map-by-field-name default, the dominant modern NFT shape (confirmed live
    // on mainnet 2026-06-17: CCHHGIOB… token_id 93, CARTUL5A… token_id 133). A
    // map without a token_id key falls through to the caller, which skips
    // known-fungible maps and tripwires the rest.
    if remaining_topics.len() >= n_addrs
        && let Some(token_id) = map_get(data, "token_id")
        && looks_like_token_id(token_id)
        && let Some(addrs) = collect_addresses(&remaining_topics[..n_addrs])
    {
        return Some((addrs, token_id.clone()));
    }

    None
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

    /// Mainnet-defaulting shadow of [`detect_nft_events`] so the existing
    /// fixtures (all mainnet) keep their one-arg call shape after task 0294
    /// added the `net_id` SAC gate. Shadows the glob-imported production fn;
    /// the real two-arg fn stays reachable as `super::detect_nft_events`.
    fn detect_nft_events(events: &[ExtractedEvent]) -> Vec<NftEvent> {
        super::detect_nft_events(
            events,
            &crate::sac::network_id(crate::sac::MAINNET_PASSPHRASE),
        )
    }
    use serde_json::json;

    fn make_event(contract_id: &str, topics: Vec<Value>, data: Value) -> ExtractedEvent {
        ExtractedEvent {
            transaction_hash: "abcd1234".into(),
            event_type: ContractEventType::Contract,
            source: EventSource::PerOp,
            contract_id: Some(contract_id.into()),
            topics: json!(topics),
            data,
            event_index: 0,
            op_index: None,
            event_pos_in_op: None,
            stage: None,
            ledger_sequence: 100,
            created_at: 1700000000,
        }
    }

    // ---- task 0323: detect_undeployed_sac_overrides ----

    const USDC_SAC: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";
    // SEP-11 asset string whose SAC derives to USDC_SAC on mainnet.
    const USDC_ASSET: &str = "USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";

    fn sac_event(emitter: &str, signature: &str) -> ExtractedEvent {
        make_event(
            emitter,
            vec![
                json!({"type": "sym", "value": signature}),
                json!({"type": "address", "value": "GFROM"}),
                json!({"type": "address", "value": "GTO"}),
                json!({"type": "string", "value": USDC_ASSET}),
            ],
            json!({"type": "i128", "value": "1000000"}),
        )
    }

    #[test]
    fn undeployed_sac_overrides_collects_and_dedups() {
        let net = crate::sac::network_id(crate::sac::MAINNET_PASSPHRASE);
        // tx1: two crypto-proven USDC-SAC events (transfer + mint) → ONE override.
        // tx2: a bespoke transfer whose last topic is an address, not a SEP-11
        // asset → the gate rejects it (no override).
        let bespoke = make_event(
            "CBESPOKE",
            vec![
                json!({"type": "sym", "value": "transfer"}),
                json!({"type": "address", "value": "GFROM"}),
                json!({"type": "address", "value": "GTO"}),
            ],
            json!({"type": "u32", "value": 42}),
        );
        let events = vec![
            (
                "tx1".to_string(),
                vec![sac_event(USDC_SAC, "transfer"), sac_event(USDC_SAC, "mint")],
            ),
            ("tx2".to_string(), vec![bespoke]),
        ];
        let mut ovs = super::detect_undeployed_sac_overrides(&events, &net);
        assert_eq!(ovs.len(), 1, "one override, deduped across two SAC events");
        let ov = ovs.pop().unwrap();
        assert_eq!(ov.contract_id, USDC_SAC);
        assert!(
            matches!(ov.identity, crate::types::SacAssetIdentity::Credit { ref code, .. } if code == "USDC"),
            "identity carries the classic asset for the AC#3 assets row",
        );
    }

    #[test]
    fn undeployed_sac_overrides_skips_diagnostic() {
        let net = crate::sac::network_id(crate::sac::MAINNET_PASSPHRASE);
        let mut diag = sac_event(USDC_SAC, "transfer");
        diag.source = EventSource::Diagnostic;
        let events = vec![("tx".to_string(), vec![diag])];
        assert!(
            super::detect_undeployed_sac_overrides(&events, &net).is_empty(),
            "diagnostic-container SAC events are skipped (byte-identical mirrors)",
        );
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
    fn sac_transfer_is_gated_out_not_an_nft() {
        // Task 0294 — a CAP-67 classic-asset SAC transfer: asset string in the
        // LAST topic, i128 AMOUNT in data, emitter == derive_sac(asset). The
        // gate proves it is a fungible amount and drops it before the i128 can
        // be mis-read as an NFT token_id. (USDC mainnet SAC + issuer.)
        const USDC_SAC: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";
        let event = make_event(
            USDC_SAC,
            vec![
                json!({"type": "sym", "value": "transfer"}),
                json!({"type": "address", "value": "GFROM"}),
                json!({"type": "address", "value": "GTO"}),
                json!({"type": "string", "value": "USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"}),
            ],
            json!({"type": "i128", "value": "8441727124"}),
        );
        assert!(
            detect_nft_events(&[event]).is_empty(),
            "a crypto-proven SAC transfer must never be detected as an NFT"
        );
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
    fn parser_emits_i128_transfer_as_nft_candidate() {
        // The parser is deliberately permissive on the `i128` payload
        // because BOTH legit SEP-39 NFTs (e.g. mainnet collection
        // `CDA5FGE4LZP4S45LP6AJLWMLKWHVWMKFSIKVYEBSIYOB25NWLKCLL7RY`
        // — confirmed via `stellar contract fetch` on 2026-05-13) AND
        // SEP-41 fungible transfers use `i128` as their payload shape.
        // The two cases cannot be distinguished by the event payload
        // alone — only by inspecting the contract's WASM interface for
        // NFT-specific functions (`owner_of`, `token_uri`,
        // `approve_for_all`, …).
        //
        // The 0118 Patch C whitelist that briefly rejected `i128` /
        // `u128` at this layer was reverted (2026-05-13) after the
        // pre-audit re-test against live mainnet RPC found a real
        // SEP-39 NFT using `i128` for `token_id` — confirming that
        // discrimination MUST be WASM-signature-based.
        //
        // The authoritative NFT-vs-fungible decision lives in the
        // persist-time filter
        // (`crates/indexer/src/handler/persist/write.rs::resolve_nft_filter`),
        // which reads `soroban_contracts.contract_type` populated by
        // `xdr_parser::classify_contract_from_wasm_spec` when the WASM
        // upload is observed. A `Fungible` / `Token`-classified contract's
        // rows are dropped before reaching `nfts`; an `Nft`-classified
        // contract's rows go to the hot `nfts` table; `Other` / NULL go
        // to the `nfts_pending` quarantine (task 0217), to be promoted
        // when a later WASM upload flips the verdict.
        //
        // This test guards the parser contract: `i128` data must
        // produce an `NftEvent` so the filter has something to inspect.
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
        event.event_type = ContractEventType::System;

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
    fn detect_nft_transfer_with_args_packed_in_data_vec() {
        // Bachini / ERC-721-port shape: topics carry ONLY the event symbol;
        // (from, to, token_id) are packed into the data as a Vec tuple.
        // Confirmed mainnet shape for CDA5FGE4… —
        // `publish((Symbol("Transfer"),), (owner, to, token_id))`.
        let event = make_event(
            "CABC123",
            vec![json!({"type": "sym", "value": "Transfer"})],
            json!({"type": "vec", "value": [
                {"type": "address", "value": "GFROM..."},
                {"type": "address", "value": "GTO..."},
                {"type": "i128", "value": "7"},
            ]}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "transfer");
        assert_eq!(nft_events[0].from.as_deref(), Some("GFROM..."));
        assert_eq!(nft_events[0].to.as_deref(), Some("GTO..."));
        assert_eq!(nft_events[0].token_id["value"], "7");
    }

    #[test]
    fn detect_nft_mint_with_args_packed_in_data_vec() {
        // Packed shape: topics carry only the symbol; (to, token_id) in data
        // vec. Bachini: `publish((Symbol("Mint"),), (to, token_id))`.
        let event = make_event(
            "CABC123",
            vec![json!({"type": "sym", "value": "Mint"})],
            json!({"type": "vec", "value": [
                {"type": "address", "value": "GTO..."},
                {"type": "i128", "value": "9"},
            ]}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "mint");
        assert!(nft_events[0].from.is_none());
        assert_eq!(nft_events[0].to.as_deref(), Some("GTO..."));
        assert_eq!(nft_events[0].token_id["value"], "9");
    }

    #[test]
    fn detect_nft_burn_with_args_packed_in_data_vec() {
        // Packed shape: topics carry only the symbol; (from, token_id) in data.
        let event = make_event(
            "CABC123",
            vec![json!({"type": "sym", "value": "Burn"})],
            json!({"type": "vec", "value": [
                {"type": "address", "value": "GFROM..."},
                {"type": "i128", "value": "3"},
            ]}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "burn");
        assert_eq!(nft_events[0].from.as_deref(), Some("GFROM..."));
        assert!(nft_events[0].to.is_none());
        assert_eq!(nft_events[0].token_id["value"], "3");
    }

    #[test]
    fn token_id_as_extra_topic_is_dropped_and_tripwired() {
        // A token_id-as-4th-topic layout is NOT a recognised shape (no SEP
        // defines it, no on-chain instance found). It must NOT be speculatively
        // parsed — it is dropped (and the detect_nft_events tripwire logs it).
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "transfer"}),
                json!({"type": "address", "value": "GFROM..."}),
                json!({"type": "address", "value": "GTO..."}),
                json!({"type": "u64", "value": 88}),
            ],
            json!({"type": "void", "value": null}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert!(nft_events.is_empty());
    }

    #[test]
    fn map_data_transfer_is_deferred_and_dropped() {
        // map-shaped data not yet handled (key naming varies across contracts);
        // the tripwire logs it. Documents the current intentional boundary —
        // change this test when map support lands (driven by real prod samples).
        let event = make_event(
            "CABC123",
            vec![json!({"type": "sym", "value": "transfer"})],
            json!({"type": "map", "value": [
                {"key": {"type":"sym","value":"from"}, "value": {"type":"address","value":"GFROM..."}},
                {"key": {"type":"sym","value":"to"}, "value": {"type":"address","value":"GTO..."}},
                {"key": {"type":"sym","value":"token_id"}, "value": {"type":"u64","value":1}},
            ]}),
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

    #[test]
    fn detect_real_mainnet_bachini_mint_event() {
        // Ground-truth regression test against the ACTUAL on-chain event of the
        // James Bachini NFT (`CDA5FGE4LZP4S45LP6AJLWMLKWHVWMKFSIKVYEBSIYOB25NWLKCLL7RY`),
        // fetched 2026-06-16 from stellar.expert
        // (`/explorer/public/contract/{id}/events`). The contract has exactly
        // one event — a Mint — and its raw XDR is embedded verbatim below.
        //
        // Shape: topics = [Symbol("Mint")], data = Vec[Address(to), I128(id)].
        // This is the packed layout that the pre-fix parser silently dropped
        // (remaining topics empty + `looks_like_token_id(vec)` false), which is
        // why this real NFT had 0 rows. This test decodes the real XDR through
        // the same `scval_to_typed_json` the production pipeline uses and proves
        // `detect_nft_events` now surfaces it.
        use crate::scval::scval_to_typed_json;
        use stellar_xdr::{Limits, ReadXdr, ScVal};

        // Raw on-chain XDR as hex (padding-free). topic = Symbol("Mint");
        // data = Vec[Address(GB2QDU…), I128(1)].
        let dec = |hexs: &str| {
            let bytes = hex::decode(hexs).unwrap();
            ScVal::from_xdr(bytes, Limits::none()).unwrap()
        };
        let topic = dec("0000000f000000044d696e74");
        let data = dec(
            "0000001000000001000000020000001200000000000000007501d2ff7273ee0426d3d05463765504e916127fa09fb33bd35d802529d942eb0000000a00000000000000000000000000000001",
        );

        let event = ExtractedEvent {
            transaction_hash: "real-mainnet".into(),
            event_type: ContractEventType::Contract,
            source: EventSource::PerOp,
            contract_id: Some("CDA5FGE4LZP4S45LP6AJLWMLKWHVWMKFSIKVYEBSIYOB25NWLKCLL7RY".into()),
            topics: json!([scval_to_typed_json(&topic)]),
            data: scval_to_typed_json(&data),
            event_index: 0,
            op_index: None,
            event_pos_in_op: None,
            stage: None,
            ledger_sequence: 1,
            created_at: 1732801047,
        };

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1, "real Bachini Mint must be detected");
        assert_eq!(nft_events[0].event_kind, "mint");
        assert!(nft_events[0].from.is_none());
        assert_eq!(
            nft_events[0].to.as_deref(),
            Some("GB2QDUX7OJZ64BBG2PIFIY3WKUCOSFQSP6QJ7MZ32NOYAJJJ3FBOXA36"),
        );
        assert_eq!(nft_events[0].token_id["type"], "i128");
        assert_eq!(nft_events[0].token_id["value"], "1");
    }

    #[test]
    fn detect_nft_mint_with_token_id_in_data_map() {
        // Canonical OpenZeppelin / SEP-50 shape: topics = [Symbol("mint"),
        // Address(to)], data = map{"token_id": uN}. The soroban-sdk
        // `#[contractevent]` macro defaults to map-by-field-name, so this is the
        // de-facto modern NFT shape. Confirmed live on mainnet 2026-06-17 (e.g.
        // CCHHGIOB… mint token_id 93, CARTUL5A… token_id 133).
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "mint"}),
                json!({"type": "address", "value": "GTO..."}),
            ],
            json!({"type": "map", "value": [
                {"key": {"type":"sym","value":"token_id"}, "value": {"type":"u32","value":133}},
            ]}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "mint");
        assert!(nft_events[0].from.is_none());
        assert_eq!(nft_events[0].to.as_deref(), Some("GTO..."));
        assert_eq!(nft_events[0].token_id["type"], "u32");
        assert_eq!(nft_events[0].token_id["value"], 133);
    }

    #[test]
    fn detect_nft_transfer_with_token_id_in_data_map() {
        // OZ/SEP-50 transfer: topics = [transfer, from, to], data = map{token_id}.
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "transfer"}),
                json!({"type": "address", "value": "GFROM..."}),
                json!({"type": "address", "value": "GTO..."}),
            ],
            json!({"type": "map", "value": [
                {"key": {"type":"sym","value":"token_id"}, "value": {"type":"u64","value":7}},
            ]}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "transfer");
        assert_eq!(nft_events[0].from.as_deref(), Some("GFROM..."));
        assert_eq!(nft_events[0].to.as_deref(), Some("GTO..."));
        assert_eq!(nft_events[0].token_id["value"], 7);
    }

    #[test]
    fn detect_nft_burn_with_token_id_in_data_map() {
        // OZ/SEP-50 burn: topics = [burn, from], data = map{token_id}.
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "burn"}),
                json!({"type": "address", "value": "GFROM..."}),
            ],
            json!({"type": "map", "value": [
                {"key": {"type":"sym","value":"token_id"}, "value": {"type":"u32","value":42}},
            ]}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].event_kind, "burn");
        assert_eq!(nft_events[0].from.as_deref(), Some("GFROM..."));
        assert!(nft_events[0].to.is_none());
        assert_eq!(nft_events[0].token_id["value"], 42);
    }

    #[test]
    fn token_id_in_map_alongside_other_keys_still_parses() {
        // A map carrying token_id plus unrelated keys (e.g. position-NFT mints)
        // must still resolve token_id by key, ignoring the extra fields.
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "mint"}),
                json!({"type": "address", "value": "GTO..."}),
            ],
            json!({"type": "map", "value": [
                {"key": {"type":"sym","value":"amount0"}, "value": {"type":"i128","value":"100"}},
                {"key": {"type":"sym","value":"token_id"}, "value": {"type":"u32","value":5}},
            ]}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert_eq!(nft_events.len(), 1);
        assert_eq!(nft_events[0].token_id["value"], 5);
    }

    #[test]
    fn fungible_map_amount_is_not_an_nft() {
        // CAP-67 / SAC muxed mint: topics = [mint, to], data =
        // map{amount, to_muxed_id}. Chain-wide there are ~148M of these vs ~890
        // NFT map{token_id} mints — a map WITHOUT a token_id key must NOT be
        // ingested as an NFT (and must not spam the tripwire).
        let event = make_event(
            "CABC123",
            vec![
                json!({"type": "sym", "value": "mint"}),
                json!({"type": "address", "value": "GTO..."}),
            ],
            json!({"type": "map", "value": [
                {"key": {"type":"sym","value":"amount"}, "value": {"type":"i128","value":"5000"}},
                {"key": {"type":"sym","value":"to_muxed_id"}, "value": {"type":"u64","value":140482179}},
            ]}),
        );

        let nft_events = detect_nft_events(&[event]);
        assert!(nft_events.is_empty());
    }

    #[test]
    fn consecutive_mint_map_range_expands_to_n_mints() {
        // OZ Consecutive (EIP-2309 analog): topics = [consecutive_mint, to],
        // data = map{from_token_id, to_token_id} — one event mints the whole
        // inclusive range. Real mainnet shape (2026-06-17).
        let event = make_event(
            "CABC123",
            vec![
                json!({"type":"sym","value":"consecutive_mint"}),
                json!({"type":"address","value":"GTO..."}),
            ],
            json!({"type":"map","value":[
                {"key":{"type":"sym","value":"from_token_id"},"value":{"type":"u32","value":0}},
                {"key":{"type":"sym","value":"to_token_id"},"value":{"type":"u32","value":2}},
            ]}),
        );

        let nft = detect_nft_events(&[event]);
        assert_eq!(nft.len(), 3);
        assert!(nft.iter().all(|e| e.event_kind == "mint"
            && e.from.is_none()
            && e.to.as_deref() == Some("GTO...")));
        assert_eq!(nft[0].token_id["value"], 0);
        assert_eq!(nft[1].token_id["value"], 1);
        assert_eq!(nft[2].token_id["value"], 2);
    }

    #[test]
    fn consecutive_mint_vec_range_expands_to_n_mints() {
        // Alternate encoding also seen on mainnet: data = vec[from, to].
        let event = make_event(
            "CABC123",
            vec![
                json!({"type":"sym","value":"consecutive_mint"}),
                json!({"type":"address","value":"GTO..."}),
            ],
            json!({"type":"vec","value":[{"type":"u32","value":5},{"type":"u32","value":6}]}),
        );

        let nft = detect_nft_events(&[event]);
        assert_eq!(nft.len(), 2);
        assert_eq!(nft[0].token_id["value"], 5);
        assert_eq!(nft[1].token_id["value"], 6);
    }

    #[test]
    fn consecutive_mint_single_token() {
        let event = make_event(
            "CABC123",
            vec![
                json!({"type":"sym","value":"consecutive_mint"}),
                json!({"type":"address","value":"GTO..."}),
            ],
            json!({"type":"map","value":[
                {"key":{"type":"sym","value":"from_token_id"},"value":{"type":"u32","value":0}},
                {"key":{"type":"sym","value":"to_token_id"},"value":{"type":"u32","value":0}},
            ]}),
        );

        let nft = detect_nft_events(&[event]);
        assert_eq!(nft.len(), 1);
        assert_eq!(nft[0].token_id["value"], 0);
    }

    #[test]
    fn consecutive_mint_inverted_range_is_dropped() {
        // to < from is malformed — must drop (and tripwire), never underflow or
        // emit a giant range.
        let event = make_event(
            "CABC123",
            vec![
                json!({"type":"sym","value":"consecutive_mint"}),
                json!({"type":"address","value":"GTO..."}),
            ],
            json!({"type":"vec","value":[{"type":"u32","value":9},{"type":"u32","value":3}]}),
        );

        let nft = detect_nft_events(&[event]);
        assert!(nft.is_empty());
    }

    #[test]
    fn consecutive_mint_oversized_range_is_dropped() {
        // A span at/above MAX_CONSECUTIVE_RANGE is dropped + tripwired, never
        // expanded — guards the (Lambda) indexer against a hostile/buggy range.
        let event = make_event(
            "CABC123",
            vec![
                json!({"type":"sym","value":"consecutive_mint"}),
                json!({"type":"address","value":"GTO..."}),
            ],
            json!({"type":"vec","value":[{"type":"u32","value":0},{"type":"u32","value":65535}]}),
        );

        let nft = detect_nft_events(&[event]);
        assert!(nft.is_empty());
    }

    #[test]
    fn detect_real_mainnet_map_token_id_mint() {
        // Ground-truth regression against the ACTUAL on-chain event of an
        // OpenZeppelin/SEP-50 NFT (CARTUL5A…, ledger 62,952,436, fetched
        // 2026-06-17 from mainnet RPC getEvents). Shape: topics = [Symbol("mint"),
        // Address(to)], data = map{"token_id": u32}. This canonical map shape is
        // exactly what the pre-fix parser dropped (`looks_like_token_id(map)` is
        // false). Decodes the real XDR through the production `scval_to_typed_json`
        // and proves Shape C now surfaces it.
        use crate::scval::scval_to_typed_json;
        use stellar_xdr::{Limits, ReadXdr, ScVal};
        let dec = |h: &str| ScVal::from_xdr(hex::decode(h).unwrap(), Limits::none()).unwrap();
        let topic0 = dec("0000000f000000046d696e74");
        let topic1 = dec(
            "0000001200000000000000000f62885810e5226331ad79ce79411bfd7d263a9f7d3371c60365b04b6d1216e1",
        );
        let data = dec("0000001100000001000000010000000f00000008746f6b656e5f69640000000300000085");

        let event = ExtractedEvent {
            transaction_hash: "real-mainnet-map".into(),
            event_type: ContractEventType::Contract,
            source: EventSource::PerOp,
            contract_id: Some("CARTUL5AWDZYBSN7HUUJZSKCAKCIAKM7M54Z76G6KRYCK4XPR3OHUQZ4".into()),
            topics: json!([scval_to_typed_json(&topic0), scval_to_typed_json(&topic1)]),
            data: scval_to_typed_json(&data),
            event_index: 0,
            op_index: None,
            event_pos_in_op: None,
            stage: None,
            ledger_sequence: 62952436,
            created_at: 1700000000,
        };

        // The real data really is a map carrying token_id — the pre-fix drop reason.
        assert_eq!(event.data["type"], "map");

        let nft = detect_nft_events(&[event]);
        assert_eq!(nft.len(), 1, "real map{{token_id}} mint must be detected");
        assert_eq!(nft[0].event_kind, "mint");
        assert!(nft[0].from.is_none());
        assert_eq!(
            nft[0].to.as_deref(),
            Some("GAHWFCCYCDSSEYZRVV4446KBDP6X2JR2T56TG4OGANS3AS3NCILOCGU5"),
        );
        assert_eq!(nft[0].token_id["type"], "u32");
        assert_eq!(nft[0].token_id["value"], 133);
    }

    #[test]
    fn detect_real_mainnet_consecutive_mint_range() {
        // Ground-truth: an ACTUAL on-chain consecutive_mint (OZ Consecutive) from
        // CAKSC7JH… (mainnet, captured 2026-06-17 from prod soroban_events).
        // topics = [Symbol("consecutive_mint"), Address(to)],
        // data = vec[u32 4, u32 149] — an inclusive range of 146 tokens in ONE
        // event. The pre-fix parser dropped it (consecutive_mint was not a
        // recognised symbol). Raw XDR is unavailable (>7-day RPC retention), so
        // this uses the real decoded event values verbatim from prod CH.
        let event = ExtractedEvent {
            transaction_hash: "real-mainnet-consecutive".into(),
            event_type: ContractEventType::Contract,
            source: EventSource::PerOp,
            contract_id: Some("CAKSC7JHQFBJ4LIYOJQGJX2URGGWABX2WM6OZ5WQVK57VNRUG4DUYK7F".into()),
            topics: serde_json::from_str(
                r#"[{"type":"sym","value":"consecutive_mint"},{"type":"address","value":"GBWHGYD5DFPQMJSUEEA77IT7YJ75PYQQFOCMP7HT5OIF2ULKJK22N4J4"}]"#,
            )
            .unwrap(),
            data: serde_json::from_str(
                r#"{"type":"vec","value":[{"type":"u32","value":4},{"type":"u32","value":149}]}"#,
            )
            .unwrap(),
            event_index: 0,
            op_index: None,
            event_pos_in_op: None,
            stage: None,
            ledger_sequence: 1,
            created_at: 1700000000,
        };

        let nft = detect_nft_events(&[event]);
        assert_eq!(
            nft.len(),
            146,
            "consecutive_mint [4,149] expands to 146 mints"
        );
        assert!(nft.iter().all(|e| e.event_kind == "mint"
            && e.from.is_none()
            && e.to.as_deref()
                == Some("GBWHGYD5DFPQMJSUEEA77IT7YJ75PYQQFOCMP7HT5OIF2ULKJK22N4J4")));
        assert_eq!(nft[0].token_id["value"], 4);
        assert_eq!(nft[145].token_id["value"], 149);
    }
}
