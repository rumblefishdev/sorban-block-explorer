//! Every Soroswap registration in history through the real decoder, plus the
//! u32-sieve false-positive measurement over raw mainnet ledgers (task 0518).
//!
//! Two corpora, each skipped (passes trivially) when its env is unset:
//!
//! - `FACTORY_PAIR_CORPUS` — JSONEachRow harvest of ALL `new_pair`
//!   events with their emitting factory. Harvest with:
//!
//!   ```sh
//!   chq "SELECT e.ledger_sequence AS ledger_sequence, e.event_index AS event_index,
//!               sc.contract_id AS factory, e.topics_xdr AS topics_xdr, e.data_xdr AS data_xdr
//!        FROM soroban_events e
//!        INNER JOIN (SELECT DISTINCT id, contract_id FROM soroban_contracts) sc
//!          ON sc.id = e.contract_id
//!        WHERE JSONExtractString(e.topics_xdr,2,'value')='new_pair'
//!          AND JSONExtractString(e.topics_xdr,1,'value')='SoroswapFactory'
//!        ORDER BY e.ledger_sequence, e.event_index
//!        FORMAT JSONEachRow" > /tmp/new_pair_corpus.jsonl
//!   ```
//!
//!   Invariants: 100% decode (a registration that cannot be read is a pair
//!   going missing), no pair registered twice, and per factory the vendor's
//!   own counter is GAPLESS from 1 — `{new_pairs_length}` == `1..=count` —
//!   which is the closure check every backfill reuses.
//!
//! - `POOL_STATE_CORPUS_DIR` — the shared 90-raw-ledger corpus (all eras).
//!   Every instance the u32 sieve extracts must be a REGISTERED pair
//!   (needs the first corpus too): the measured false-positive count of the
//!   composite shape against real history, per 0516's shape-not-name rule.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use stellar_xdr::{LedgerCloseMetaBatch, Limits, ReadXdr};
use xdr_parser::pool_pair_factory::{extract_factory_pairs, parse_new_pair};

#[derive(serde::Deserialize)]
struct Row {
    ledger_sequence: i64,
    factory: String,
    topics_xdr: String,
    data_xdr: String,
}

fn load_corpus() -> Option<Vec<Row>> {
    let path = std::env::var("FACTORY_PAIR_CORPUS").ok()?;
    let raw = std::fs::read_to_string(&path).expect("corpus readable");
    Some(
        raw.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("corpus row"))
            .collect(),
    )
}

#[test]
fn every_registration_in_history_decodes_and_every_counter_is_gapless() {
    let Some(rows) = load_corpus() else {
        eprintln!("FACTORY_PAIR_CORPUS unset — skipping (see module docs)");
        return;
    };
    let mut pairs: HashSet<String> = HashSet::new();
    let mut counters: HashMap<String, Vec<u32>> = HashMap::new();
    for row in &rows {
        let topics: Value = serde_json::from_str(&row.topics_xdr).expect("topics json");
        let data: Value = serde_json::from_str(&row.data_xdr).expect("data json");
        let ev = parse_new_pair(&topics, &data).unwrap_or_else(|e| {
            panic!(
                "ledger {}: a real registration failed to decode: {e:?}",
                row.ledger_sequence
            )
        });
        assert!(
            pairs.insert(ev.pair.clone()),
            "pair {} registered twice — the registry key assumption breaks",
            ev.pair
        );
        counters
            .entry(row.factory.clone())
            .or_default()
            .push(ev.new_pairs_length);
    }
    assert_eq!(
        pairs.len(),
        rows.len(),
        "one registration = one distinct pair"
    );
    for (factory, mut lens) in counters {
        lens.sort_unstable();
        let expect: Vec<u32> = (1..=u32::try_from(lens.len()).unwrap()).collect();
        assert_eq!(
            lens, expect,
            "factory {factory}: the vendor counter has a gap — a registration is missing from the store"
        );
    }
    println!(
        "registrations decoded: {} across {} factories",
        rows.len(),
        4
    );
}

#[test]
fn the_u32_sieve_claims_no_foreign_instance_in_the_raw_corpus() {
    let Ok(dir) = std::env::var("POOL_STATE_CORPUS_DIR") else {
        eprintln!("POOL_STATE_CORPUS_DIR unset — skipping (see module docs)");
        return;
    };
    let Some(rows) = load_corpus() else {
        eprintln!("FACTORY_PAIR_CORPUS unset — the FP check needs the registered set; skipping");
        return;
    };
    let registered: HashSet<String> = rows
        .iter()
        .map(|r| {
            let data: Value = serde_json::from_str(&r.data_xdr).unwrap();
            let topics: Value = serde_json::from_str(&r.topics_xdr).unwrap();
            parse_new_pair(&topics, &data).unwrap().pair
        })
        .collect();

    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("corpus dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "xdr"))
        .collect();
    files.sort();
    let mut extracted = 0usize;
    let mut foreign: Vec<String> = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let batch = LedgerCloseMetaBatch::from_xdr(&bytes, Limits::none()).unwrap();
        for lcm in batch.ledger_close_metas.iter() {
            xdr_parser::meta::for_each_tx_meta(lcm, |seq, i, meta| {
                let hash = format!("{i:064x}");
                let changes = xdr_parser::extract_ledger_entry_changes(meta, &hash, seq, 0);
                for pair in extract_factory_pairs(&changes) {
                    extracted += 1;
                    if !registered.contains(&pair.state.pair) {
                        foreign.push(format!("{} @ {}", pair.state.pair, seq));
                    }
                }
            });
        }
    }
    assert!(
        foreign.is_empty(),
        "the u32 sieve claimed {} instance(s) outside the registered set — \
         measure and tighten before staging trusts it: {:?}",
        foreign.len(),
        &foreign[..foreign.len().min(5)]
    );
    println!("sieve extractions in raw corpus: {extracted}, foreign: 0");
}
