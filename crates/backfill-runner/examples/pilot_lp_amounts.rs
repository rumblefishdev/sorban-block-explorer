//! Task 0279 backfill pilot: how long one ledger costs and how many
//! `lp_operation_amounts` rows it yields.
//!
//! Runs the REAL backfill inner loop — `parse_ledger` + `stage::prepare` — over
//! `.xdr.zst` files already on disk, so the number it reports is the per-ledger
//! CPU cost the re-parse actually pays. Deliberately excludes the two costs
//! that are environment-specific and measured separately: the S3 fetch (the
//! runner pre-fetches with s5cmd) and the ClickHouse insert.
//!
//! No DB, no network, no writes.
//!
//! Run:
//!
//!     cargo run --release -p backfill-runner --example pilot_lp_amounts -- <DIR>

use std::collections::HashMap;
use std::time::Instant;

use db_clickhouse::persist::stage::{self, StageInputs};
use xdr_parser::{decompress_zstd, deserialize_batch};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: pilot_lp_amounts <DIR>");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "zst"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .xdr.zst files in {dir}");

    let mut ledgers = 0usize;
    let mut rows = 0usize;
    let mut with_rows = 0usize;
    let mut bytes = 0u64;
    let started = Instant::now();

    for path in &files {
        let raw = std::fs::read(path).expect("read file");
        bytes += raw.len() as u64;
        let batch = deserialize_batch(&decompress_zstd(&raw).expect("unzstd")).expect("xdr");
        for meta in batch.ledger_close_metas.iter() {
            let parsed = indexer::handler::process::parse_ledger(meta);
            let staged = stage::prepare_with_sac_overrides(&StageInputs {
                ledger: &parsed.ledger,
                transactions: &parsed.transactions,
                operations: &parsed.operations,
                events: &parsed.events,
                invocations: &parsed.invocations,
                contract_interfaces: &parsed.contract_interfaces,
                contract_deployments: &parsed.contract_deployments,
                account_states: &parsed.account_states,
                liquidity_pools: &parsed.liquidity_pools,
                pool_snapshots: &parsed.pool_snapshots,
                pool_family_writes: &parsed.pool_family_writes,
                assets: &parsed.assets,
                nfts: &parsed.nfts,
                nft_events: &parsed.nft_events,
                lp_positions: &parsed.lp_positions,
                contract_metadata_writes: &parsed.contract_metadata_writes,
                soroban_token_balances: &parsed.soroban_token_balances,
                // Empty for the pilot: SAC re-keying touches balances, never
                // the LP amounts this measures.
                sac_classic: &HashMap::new(),
                sac_overrides: &parsed.sac_overrides,
                prior_wasm_verdicts: &HashMap::new(),
                prior_contract_verdicts: &HashMap::new(),
                prior_contract_rows: &HashMap::new(),
            })
            .expect("stage");
            ledgers += 1;
            rows += staged.lp_amount_rows.len();
            if !staged.lp_amount_rows.is_empty() {
                with_rows += 1;
            }
        }
    }

    let secs = started.elapsed().as_secs_f64();
    println!("ledgers            {ledgers}");
    println!("lp_amount rows     {rows}");
    println!("rows / ledger      {:.1}", rows as f64 / ledgers as f64);
    println!(
        "ledgers with rows  {with_rows} ({:.0}%)",
        100.0 * with_rows as f64 / ledgers as f64
    );
    println!("input bytes        {bytes}");
    println!("wall seconds       {secs:.2}");
    println!("ledgers / sec      {:.1}", ledgers as f64 / secs);
}
