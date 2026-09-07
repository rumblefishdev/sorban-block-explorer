//! Every `add_pool` event on mainnet, run through the real decoder (task 0374).
//!
//! The inline tests in `pool_router.rs` pin four hand-copied on-chain payloads.
//! Useful, but four samples cannot support the claim this module makes — that
//! **497 of 497** registrations in all history decode. That number was first
//! measured with a ClickHouse query *imitating* the decoder's checks, which is
//! not the same thing as running the decoder. This test runs the decoder.
//!
//! The population is bounded and complete: `add_pool` is the entire pool
//! registry, so this is not a sample. Every row must decode, and every decoded
//! row must satisfy invariants that hold regardless of which deployment
//! emitted it. A future payload change surfaces here as a failure rather than
//! as pools quietly going missing.
//!
//! Skipped (passes trivially) when `ADD_POOL_CORPUS` is unset — CI does not
//! ship the prod harvest, matching `nft_real_corpus`. Harvest and run with:
//!
//! ```sh
//! chq "SELECT ledger_sequence, event_index, topics_xdr, data_xdr
//!      FROM soroban_events WHERE signature='add_pool'
//!      ORDER BY ledger_sequence, event_index FORMAT JSONEachRow" > /tmp/add_pool_corpus.jsonl
//!
//! ADD_POOL_CORPUS=/tmp/add_pool_corpus.jsonl \
//!   cargo test -p xdr-parser --test pool_router_real_corpus -- --nocapture
//! ```

use std::collections::BTreeMap;

use serde_json::Value;
use xdr_parser::pool_router::parse_add_pool;

/// One harvested prod row. `topics_xdr` / `data_xdr` are JSON **strings**
/// holding the decoded typed-JSON, re-parsed below.
#[derive(serde::Deserialize)]
struct Row {
    ledger_sequence: i64,
    event_index: i64,
    topics_xdr: String,
    data_xdr: String,
}

#[test]
fn every_mainnet_add_pool_decodes() {
    let Ok(path) = std::env::var("ADD_POOL_CORPUS") else {
        eprintln!("ADD_POOL_CORPUS unset — skipping (see module docs to harvest)");
        return;
    };
    let raw = std::fs::read_to_string(&path).expect("corpus file readable");

    let mut total = 0usize;
    let mut by_shape: BTreeMap<(String, usize, usize), usize> = BTreeMap::new();
    let mut rejected: Vec<String> = Vec::new();

    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        total += 1;
        let row: Row = serde_json::from_str(line).expect("corpus row parses");
        let where_ = format!("ledger {} event {}", row.ledger_sequence, row.event_index);

        let topics: Value = serde_json::from_str(&row.topics_xdr).expect("topics json");
        let data: Value = serde_json::from_str(&row.data_xdr).expect("data json");

        let ev = match parse_add_pool(&topics, &data) {
            Ok(ev) => ev,
            Err(reason) => {
                rejected.push(format!("{where_}: {reason:?}"));
                continue;
            }
        };

        // Invariants that must hold for every registration, from any
        // deployment, in any era.
        assert!(
            ev.pool.starts_with('C') && ev.pool.len() == 56,
            "{where_}: pool is not a contract address: {}",
            ev.pool
        );
        assert!(!ev.pool_type.is_empty(), "{where_}: empty pool type");
        assert!(!ev.tokens.is_empty(), "{where_}: no legs");
        for t in &ev.tokens {
            assert!(
                t.starts_with('C') && t.len() == 56,
                "{where_}: leg is not a contract address: {t}"
            );
        }
        // Legs are positional — the reserve vector is read against this order,
        // so a duplicate would make two reserves indistinguishable.
        let mut sorted = ev.tokens.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ev.tokens.len(),
            "{where_}: duplicate leg in {:?}",
            ev.tokens
        );

        *by_shape
            .entry((ev.pool_type.clone(), ev.tokens.len(), ev.init_args.len()))
            .or_default() += 1;
    }

    eprintln!("\n--- add_pool corpus: {total} events ---");
    for ((ty, legs, args), n) in &by_shape {
        eprintln!("  {ty:<13} legs={legs} init_args={args}  ->  {n}");
    }

    assert!(
        rejected.is_empty(),
        "{} of {total} registrations did not decode:\n{}",
        rejected.len(),
        rejected.join("\n")
    );
    assert!(total > 0, "corpus file was empty");
}

/// The other half of the claim: everything that is **not** a registration must
/// be refused.
///
/// The positive corpus only proves registrations decode. A decoder that
/// returned a pool for every event would pass it just as well. This runs a
/// cross-section of real mainnet traffic — every signature present in a
/// contiguous ledger window, including the router family's *own* other events
/// (`trade`, `update_reserves`, `pool_state`) and other protocols reusing
/// familiar names — and requires that not one of them yields a pool.
///
/// A single `Ok` here is a false positive: a junk row in the pool registry.
///
/// ```sh
/// chq "SELECT ledger_sequence, event_index, coalesce(signature,'<null>') AS sig,
///      topics_xdr, data_xdr FROM soroban_events
///      WHERE ledger_sequence BETWEEN 64132000 AND 64132040
///      FORMAT JSONEachRow" > /tmp/neg_raw.jsonl   # then sample per signature
///
/// NOT_ADD_POOL_CORPUS=/tmp/not_add_pool_corpus.jsonl \
///   cargo test -p xdr-parser --test pool_router_real_corpus -- --nocapture
/// ```
#[test]
fn no_other_mainnet_event_is_mistaken_for_a_registration() {
    let Ok(path) = std::env::var("NOT_ADD_POOL_CORPUS") else {
        eprintln!("NOT_ADD_POOL_CORPUS unset — skipping (see the test docs to harvest)");
        return;
    };
    let raw = std::fs::read_to_string(&path).expect("corpus file readable");

    let mut total = 0usize;
    let mut signatures: BTreeMap<String, usize> = BTreeMap::new();
    let mut false_positives: Vec<String> = Vec::new();

    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        total += 1;
        let row: NegRow = serde_json::from_str(line).expect("corpus row parses");
        *signatures.entry(row.sig.clone()).or_default() += 1;

        let topics: Value = serde_json::from_str(&row.topics_xdr).expect("topics json");
        let data: Value = serde_json::from_str(&row.data_xdr).expect("data json");

        if let Ok(ev) = parse_add_pool(&topics, &data) {
            false_positives.push(format!(
                "ledger {} event {} (signature {}) decoded as pool {}",
                row.ledger_sequence, row.event_index, row.sig, ev.pool
            ));
        }
    }

    eprintln!(
        "\n--- non-registration corpus: {total} events, {} signatures ---",
        signatures.len()
    );
    for (sig, n) in &signatures {
        eprintln!("  {sig:<20} {n}");
    }

    assert!(
        false_positives.is_empty(),
        "{} false positive(s) out of {total}:\n{}",
        false_positives.len(),
        false_positives.join("\n")
    );
    assert!(total > 0, "corpus file was empty");
}

/// Same harvest shape as [`Row`], plus the signature so failures name it.
#[derive(serde::Deserialize)]
struct NegRow {
    ledger_sequence: i64,
    event_index: i64,
    sig: String,
    topics_xdr: String,
    data_xdr: String,
}
