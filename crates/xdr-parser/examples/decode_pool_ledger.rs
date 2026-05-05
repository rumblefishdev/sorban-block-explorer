//! Lore-0189 investigation: dump every LedgerEntryChange touching a
//! specific pool_id in a single ledger XDR file.
//!
//! Used to determine whether a pool referenced by an `lp_positions` insert
//! appears in the same ledger as a `liquidity_pool` LedgerEntryChange (and
//! with what `change_type`). The answer dictates whether the
//! `extract_liquidity_pools` filter loosening (Layer 3) covers the
//! reproducer or whether the sentinel placeholder path must fire.
//!
//! Run:
//!
//!     cargo run -p xdr-parser --example decode_pool_ledger -- \
//!         --file "/Volumes/Extreme SSD 2TB/sbe-backfill-temp/FC4BC1FF--62144000-62207999/FC4BB25C--62148003.xdr.zst" \
//!         --pool d63184d4e5601fad174d9d5fa8e79f2366f6818892e43867a952e8adb13fa561
//!
//! No DB, no network — pure file scan + XDR parse + change enumeration.

use std::fs;
use std::path::PathBuf;

use stellar_xdr::curr::LedgerCloseMeta;
use xdr_parser::{decompress_zstd, deserialize_batch, extract_ledger_entry_changes};

struct Args {
    file: PathBuf,
    pool_id_hex: String,
}

fn parse_args() -> Args {
    let mut file: Option<PathBuf> = None;
    let mut pool_id: Option<String> = None;
    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--file" => file = iter.next().map(PathBuf::from),
            "--pool" => pool_id = iter.next(),
            "-h" | "--help" => {
                eprintln!("usage: decode_pool_ledger --file <PATH> --pool <POOL_ID_HEX_64>");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }
    Args {
        file: file.expect("--file is required"),
        pool_id_hex: pool_id.expect("--pool is required").to_lowercase(),
    }
}

fn ledger_metas(meta: &LedgerCloseMeta) -> Vec<(u32, Vec<TxView>)> {
    match meta {
        LedgerCloseMeta::V0(v) => vec![(
            v.ledger_header.header.ledger_seq,
            v.tx_processing
                .iter()
                .map(|p| TxView {
                    hash: hex::encode(p.result.transaction_hash.0),
                    meta: p.tx_apply_processing.clone(),
                })
                .collect(),
        )],
        LedgerCloseMeta::V1(v) => vec![(
            v.ledger_header.header.ledger_seq,
            v.tx_processing
                .iter()
                .map(|p| TxView {
                    hash: hex::encode(p.result.transaction_hash.0),
                    meta: p.tx_apply_processing.clone(),
                })
                .collect(),
        )],
        LedgerCloseMeta::V2(v) => vec![(
            v.ledger_header.header.ledger_seq,
            v.tx_processing
                .iter()
                .map(|p| TxView {
                    hash: hex::encode(p.result.transaction_hash.0),
                    meta: p.tx_apply_processing.clone(),
                })
                .collect(),
        )],
    }
}

struct TxView {
    hash: String,
    meta: stellar_xdr::curr::TransactionMeta,
}

/// Lower-case the supplied JSON value if it is a string, else return as-is.
fn json_str_lower(v: Option<&serde_json::Value>) -> Option<String> {
    v.and_then(|v| v.as_str()).map(|s| s.to_lowercase())
}

fn main() {
    let args = parse_args();
    eprintln!(
        "[decode-pool-ledger] file={} pool={}",
        args.file.display(),
        args.pool_id_hex
    );

    let bytes = fs::read(&args.file).expect("read file");
    let xdr = decompress_zstd(&bytes).expect("decompress");
    let batch = deserialize_batch(&xdr).expect("deserialize batch");

    let mut hits = 0usize;
    let mut total_changes = 0usize;

    for meta in batch.ledger_close_metas.iter() {
        for (ledger_seq, txs) in ledger_metas(meta) {
            for tx in txs {
                let changes = extract_ledger_entry_changes(
                    &tx.meta, &tx.hash, ledger_seq, /* created_at */ 0,
                );
                total_changes += changes.len();
                for c in &changes {
                    let lp_match = c.entry_type == "liquidity_pool"
                        && c.data
                            .as_ref()
                            .and_then(|d| d.get("pool_id"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_lowercase() == args.pool_id_hex)
                            .unwrap_or(false);

                    let trustline_match = c.entry_type == "trustline" && {
                        // 'created'/'updated'/'restored' carries the asset under data.asset
                        // 'removed' carries it under key.asset
                        let asset_in_data = c.data.as_ref().and_then(|d| d.get("asset"));
                        let asset_in_key = c.key.get("asset");
                        let asset = asset_in_data.or(asset_in_key);
                        asset
                            .and_then(|a| a.get("pool_id"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_lowercase() == args.pool_id_hex)
                            .unwrap_or(false)
                    };

                    if lp_match || trustline_match {
                        hits += 1;
                        let kind = if lp_match { "POOL" } else { "TRUSTLINE" };
                        println!();
                        println!("=== HIT {hits} — {kind} ===");
                        println!("  ledger_sequence: {}", c.ledger_sequence);
                        println!("  tx_hash:         {}", c.transaction_hash);
                        println!(
                            "  op_index:        {:?}",
                            c.operation_index
                                .map(|i| i.to_string())
                                .unwrap_or_else(|| "(tx-level)".to_string())
                        );
                        println!("  change_index:    {}", c.change_index);
                        println!("  entry_type:      {}", c.entry_type);
                        println!("  change_type:     {}", c.change_type);
                        println!(
                            "  key:             {}",
                            serde_json::to_string_pretty(&c.key)
                                .unwrap_or_else(|_| "<unprintable>".to_string())
                        );
                        if let Some(d) = &c.data {
                            println!(
                                "  data:            {}",
                                serde_json::to_string_pretty(d)
                                    .unwrap_or_else(|_| "<unprintable>".to_string())
                            );
                        } else {
                            println!("  data:            <none — change is `removed`>");
                        }
                        let _ = json_str_lower; // silence unused if compiler complains
                    }
                }
            }
        }
    }

    println!();
    println!("=== SUMMARY ===");
    println!("  total ledger_entry_changes scanned: {}", total_changes);
    println!("  hits for pool {}: {}", args.pool_id_hex, hits);
    if hits == 0 {
        println!();
        println!("  Pool not present in this ledger as either `liquidity_pool` entry");
        println!("  change OR nested in any `trustline.asset` field. Outcome B —");
        println!("  sentinel placeholder is the only path to preserve the position.");
    } else {
        println!();
        println!("  Inspect change_type per HIT — `state` means Layer 3 filter");
        println!("  loosening will capture the parent pool. `created/updated/restored`");
        println!("  means it would already be captured (puzzling — investigate further).");
    }
}
