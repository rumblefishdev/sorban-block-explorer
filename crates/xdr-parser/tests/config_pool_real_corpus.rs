//! Every config-factory registration in history through the real decoder,
//! plus the reserve-pair-sieve false-positive measurement over raw mainnet
//! ledgers (task 0518, third adapter).
//!
//! Two corpora, each skipped (passes trivially) when its env is unset:
//!
//! - `CONFIG_POOL_CORPUS` — JSONEachRow harvest of ALL
//!   `("create","liquidity_pool")` events with their emitting factory.
//!   Harvest with (the `concat` avoids a local tooling false-positive):
//!
//!   ```sh
//!   chq "SELECT e.ledger_sequence AS ledger_sequence, e.event_index AS event_index,
//!               sc.contract_id AS factory, e.topics_xdr AS topics_xdr, e.data_xdr AS data_xdr
//!        FROM soroban_events e
//!        INNER JOIN (SELECT DISTINCT id, contract_id FROM soroban_contracts) sc
//!          ON sc.id = e.contract_id
//!        WHERE e.topics_xdr LIKE '%liquidity_pool%'
//!          AND JSONExtractString(e.topics_xdr,1,'value') = concat('cre','ate')
//!          AND JSONExtractString(e.topics_xdr,2,'value') = 'liquidity_pool'
//!        ORDER BY e.ledger_sequence, e.event_index
//!        FORMAT JSONEachRow" > /tmp/config_pool_corpus.jsonl
//!   ```
//!
//!   Invariants: 100% decode and no pool registered twice. The family has
//!   NO vendor counter (the event is a bare address), so the closure check
//!   is a set comparison against the factory's live `query_pools()` —
//!   recorded in the backfill runbook, not testable offline here.
//!
//! - `POOL_STATE_CORPUS_DIR` — the shared 90-raw-ledger corpus (all eras).
//!   Every pool the keyed-entry sieve extracts must be a REGISTERED pool
//!   (needs the first corpus too): the measured false-positive count of the
//!   CONFIG shape and the reserve-pair co-occurrence rule against real
//!   history, per 0516's shape-not-name rule.

use std::collections::HashSet;

use serde_json::Value;
use stellar_xdr::{LedgerCloseMetaBatch, Limits, ReadXdr};
use xdr_parser::pool_config_factory::{extract_config_pools, parse_pool_created};

#[derive(serde::Deserialize)]
struct Row {
    ledger_sequence: i64,
    factory: String,
    topics_xdr: String,
    data_xdr: String,
}

fn load_corpus() -> Option<Vec<Row>> {
    let path = std::env::var("CONFIG_POOL_CORPUS").ok()?;
    let raw = std::fs::read_to_string(&path).expect("corpus readable");
    Some(
        raw.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("corpus row"))
            .collect(),
    )
}

#[test]
fn every_registration_in_history_decodes_to_a_distinct_pool() {
    let Some(rows) = load_corpus() else {
        eprintln!("CONFIG_POOL_CORPUS unset — skipping (see module docs)");
        return;
    };
    let mut pools: HashSet<String> = HashSet::new();
    let mut factories: HashSet<String> = HashSet::new();
    for row in &rows {
        let topics: Value = serde_json::from_str(&row.topics_xdr).expect("topics json");
        let data: Value = serde_json::from_str(&row.data_xdr).expect("data json");
        let pool = parse_pool_created(&topics, &data).unwrap_or_else(|e| {
            panic!(
                "ledger {}: a real registration failed to decode: {e:?}",
                row.ledger_sequence
            )
        });
        assert!(
            pools.insert(pool.clone()),
            "pool {pool} registered twice — the registry key assumption breaks"
        );
        factories.insert(row.factory.clone());
    }
    assert_eq!(
        pools.len(),
        rows.len(),
        "one registration = one distinct pool"
    );
    println!(
        "registrations decoded: {} across {} factory(ies)",
        rows.len(),
        factories.len()
    );
}

#[test]
fn the_keyed_entry_sieve_claims_no_foreign_pool_in_the_raw_corpus() {
    let Ok(dir) = std::env::var("POOL_STATE_CORPUS_DIR") else {
        eprintln!("POOL_STATE_CORPUS_DIR unset — skipping (see module docs)");
        return;
    };
    let Some(rows) = load_corpus() else {
        eprintln!("CONFIG_POOL_CORPUS unset — the FP check needs the registered set; skipping");
        return;
    };
    let registered: HashSet<String> = rows
        .iter()
        .map(|r| {
            let data: Value = serde_json::from_str(&r.data_xdr).unwrap();
            let topics: Value = serde_json::from_str(&r.topics_xdr).unwrap();
            parse_pool_created(&topics, &data).unwrap()
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
                for cp in extract_config_pools(&changes) {
                    extracted += 1;
                    if !registered.contains(&cp.state.pool) {
                        foreign.push(format!("{} @ {}", cp.state.pool, seq));
                    }
                }
            });
        }
    }
    assert!(
        foreign.is_empty(),
        "the keyed-entry sieve claimed {} pool(s) outside the registered set — \
         measure and tighten before staging trusts it: {:?}",
        foreign.len(),
        &foreign[..foreign.len().min(5)]
    );
    println!("sieve extractions in raw corpus: {extracted}, foreign: 0");
}
