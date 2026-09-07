//! Large real-data corpus check for `detect_nft_events` (task 0296).
//!
//! The three inline `detect_real_mainnet_*` tests in `nft.rs` pin three
//! hand-picked on-chain events. This test runs the parser over a BROAD corpus
//! harvested from prod ClickHouse (`soroban_events.topics_xdr` / `data_xdr`,
//! which are stored as decoded typed-JSON, NOT raw XDR) across thousands of
//! events and many distinct contracts — mint / transfer / burn /
//! consecutive_mint, both token_id-bearing (NFT) and amount-bearing (fungible)
//! shapes.
//!
//! It is a CHARACTERIZATION test, not a labelled one: prod has no per-event
//! NFT ground truth (that is the downstream WASM classifier's job). So it
//! asserts structural INVARIANTS that must hold for every detected event
//! regardless of contract, and prints a breakdown to stderr. The point is to
//! prove the permissive parser never panics, never emits a malformed
//! candidate, and the previously-dropped shapes (map / packed-vec /
//! consecutive_mint) now surface — over real volume, not three samples.
//!
//! Skipped (passes trivially) when `NFT_CORPUS` is unset — CI does not ship the
//! prod harvest. Generate the file with `/tmp/nft-corpus/harvest.sh` then run:
//! `NFT_CORPUS=/tmp/nft-corpus/corpus.jsonl cargo test -p xdr-parser
//! --test nft_real_corpus -- --nocapture`.

use std::collections::{BTreeMap, BTreeSet};

use domain::ContractEventType;
use serde_json::Value;
use xdr_parser::types::{EventSource, ExtractedEvent};
use xdr_parser::{MAINNET_PASSPHRASE, detect_nft_events, network_id};

/// One harvested prod row (CH `FORMAT JSONEachRow`). `topics_xdr` / `data_xdr`
/// are JSON STRINGS holding the decoded typed-JSON (re-parsed below).
#[derive(serde::Deserialize)]
struct Row {
    contract_id: i64,
    signature: String,
    topics_xdr: String,
    data_xdr: String,
}

/// A well-formed Stellar strkey: base32 alphabet (RFC4648 A-Z2-7), ≥56 chars.
/// Deliberately NOT prefix-restricted — a Soroban ScAddress (CAP-67) renders as
/// G (account), C (contract), M (muxed), L (liquidity pool) or B (claimable
/// balance); the corpus contains real L-prefixed `from` addresses on NFT-shaped
/// transfers. This test validates well-formedness, not address-type policy.
fn is_strkey(s: &str) -> bool {
    s.len() >= 56
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b))
}

#[test]
fn nft_real_corpus_invariants() {
    let Ok(path) = std::env::var("NFT_CORPUS") else {
        eprintln!("NFT_CORPUS unset — skipping prod corpus test");
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read corpus");

    let mut events = Vec::new();
    let mut sig_of: Vec<String> = Vec::new();
    let mut parse_fail = 0usize;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let row: Row = serde_json::from_str(line).expect("row json");
        let (Ok(topics), Ok(data)) = (
            serde_json::from_str::<Value>(&row.topics_xdr),
            serde_json::from_str::<Value>(&row.data_xdr),
        ) else {
            parse_fail += 1;
            continue;
        };
        sig_of.push(row.signature.clone());
        events.push(ExtractedEvent {
            transaction_hash: "corpus".into(),
            event_type: ContractEventType::Contract,
            source: EventSource::PerOp,
            contract_id: Some(row.contract_id.to_string()),
            topics,
            data,
            event_index: 0,
            op_index: None,
            event_pos_in_op: None,
            stage: None,
            ledger_sequence: 1,
            created_at: 1_700_000_000,
        });
    }
    assert!(!events.is_empty(), "corpus is empty");

    // Detect per-event so we can attribute output to its source signature.
    let mut total_detected = 0usize;
    let mut detected_by_sig: BTreeMap<String, usize> = BTreeMap::new();
    let mut input_by_sig: BTreeMap<String, usize> = BTreeMap::new();
    let mut contracts_in: BTreeSet<i64> = Default::default();
    let mut contracts_detected: BTreeSet<String> = Default::default();

    for (ev, sig) in events.iter().zip(sig_of.iter()) {
        *input_by_sig.entry(sig.clone()).or_default() += 1;
        if let Some(c) = &ev.contract_id
            && let Ok(n) = c.parse::<i64>()
        {
            contracts_in.insert(n);
        }
        let out = detect_nft_events(std::slice::from_ref(ev), &network_id(MAINNET_PASSPHRASE));
        if !out.is_empty() {
            *detected_by_sig.entry(sig.clone()).or_default() += out.len();
            total_detected += out.len();
        }
        for n in &out {
            // ---- HARD INVARIANTS: must hold for every real detected event ----
            assert!(
                matches!(n.event_kind.as_str(), "mint" | "transfer" | "burn"),
                "bad event_kind {:?} (sig {sig})",
                n.event_kind
            );
            // token_id present and carries a value
            assert!(
                n.token_id.get("value").is_some(),
                "token_id missing value: {} (sig {sig})",
                n.token_id
            );
            // direction invariants
            match n.event_kind.as_str() {
                "mint" => assert!(n.from.is_none(), "mint must have no `from`"),
                "burn" => assert!(n.to.is_none(), "burn must have no `to`"),
                "transfer" => assert!(
                    n.from.is_some() && n.to.is_some(),
                    "transfer must have both from+to"
                ),
                _ => unreachable!(),
            }
            // any emitted address must be a valid strkey
            if let Some(f) = &n.from {
                assert!(is_strkey(f), "bad from strkey {f}");
            }
            if let Some(t) = &n.to {
                assert!(is_strkey(t), "bad to strkey {t}");
            }
            contracts_detected.insert(n.contract_id.clone());
        }
    }

    eprintln!("\n==== NFT real corpus ====");
    eprintln!("rows parsed:      {}", events.len());
    eprintln!("json parse fails: {parse_fail}");
    eprintln!("distinct contracts in:       {}", contracts_in.len());
    eprintln!("distinct contracts detected: {}", contracts_detected.len());
    eprintln!("total candidates emitted:    {total_detected}");
    eprintln!("-- input events by signature --");
    for (s, c) in &input_by_sig {
        let d = detected_by_sig.get(s).copied().unwrap_or(0);
        eprintln!("  {s:<18} in={c:<6} detected_candidates={d}");
    }
    eprintln!("=========================\n");

    // consecutive_mint, when present, must expand (range -> N mints).
    if let Some(&n) = input_by_sig.get("consecutive_mint")
        && n > 0
    {
        assert!(
            detected_by_sig
                .get("consecutive_mint")
                .copied()
                .unwrap_or(0)
                >= n,
            "consecutive_mint should expand to >= input count"
        );
    }
}
