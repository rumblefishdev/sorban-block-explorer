//! Task 0182 empirical overlap check.
//!
//! For every Protocol-23 V4 transaction in a Galexie-emitted partition,
//! compare the byte-encoded events from `v4.operations[i].events`
//! (consensus per-op) against the byte-encoded events from
//! `v4.diagnostic_events` (auxiliary, not hashed). Reports:
//!
//! * how many per-op events have byte-identical copies in diagnostic_events
//!   (the duplicate-leak surface fixed by task 0182), and
//! * how many Contract-typed entries in diagnostic_events have NO match
//!   in per-op (orphans). Orphans from SUCCESSFUL txs would be the only
//!   case where dropping the container could lose consensus data — the
//!   safety-critical invariant this tool is meant to monitor.
//!
//! Run:
//!
//!     cargo run --release --example diag_overlap_check -- \
//!         --dir /path/to/.temp/FC4DB5FF--62016000-62079999 \
//!         --max 200 \
//!         --target-ledger 62016099
//!
//! No DB, no network — pure file scan + XDR parse + byte-set diff.
//! Re-run on every Galexie / stellar-host upgrade.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use stellar_xdr::curr::{
    ContractEventBody, ContractEventType, LedgerCloseMeta, Limits, TransactionMeta,
    TransactionMetaV4, TransactionResult, TransactionResultResult, WriteXdr,
};
use xdr_parser::{decompress_zstd, deserialize_batch};

#[derive(Default)]
struct Stats {
    ledgers_scanned: usize,
    v4_txs: usize,
    v4_txs_with_diag: usize,
    v4_txs_with_per_op: usize,
    v4_txs_with_both: usize,
    v4_txs_with_any_overlap: usize,
    total_per_op_events: usize,
    total_diag_events: usize,
    total_overlapping_pairs: usize,
    diag_inner_contract: usize,
    diag_inner_system: usize,
    diag_inner_diagnostic: usize,
    sample_overlap_tx: Option<String>,
    sample_per_op_count: usize,
    sample_diag_count: usize,
    sample_overlap_count: usize,
    target_ledger_breakdown: Vec<TxBreakdown>,
    /// Orphan breakdown by tx success status.
    orphan_from_failed_tx: usize,
    orphan_from_successful_tx: usize,
    /// Tx counts by tx success status (only counting V4 txs with diag).
    failed_tx_with_diag: usize,
    successful_tx_with_diag: usize,
    /// THE KEY SAFETY METRIC.
    ///
    /// "Orphan" = a Contract-typed entry in `v4.diagnostic_events` whose
    /// XDR bytes do NOT appear in `v4.operations[i].events`. If this is
    /// non-zero, then dropping the diagnostic_events container loses
    /// Contract events that no other Stellar consensus location carries.
    ///
    /// Hypothesis from soroban-host source: orphans should occur ONLY
    /// for failed contract calls (where per-op is empty but the host
    /// still emitted Contract events into the diagnostic stream before
    /// the failure was finalized).
    orphan_contract_in_diag: usize,
    orphan_cases: Vec<OrphanCase>,
}

struct TxBreakdown {
    tx_hash_hex: String,
    per_op_event_count: usize,
    diag_event_count: usize,
    diag_contract: usize,
    diag_system: usize,
    diag_diagnostic: usize,
    overlap_count: usize,
    tx_level_event_count: usize,
    tx_level_event_contracts: Vec<String>,
    per_op_event_contracts: Vec<String>,
}

#[derive(Clone)]
struct OrphanCase {
    seq: u32,
    tx_hash_hex: String,
    per_op_count: usize,
    diag_contract_count: usize,
    orphan_count: usize,
    tx_successful: bool,
    result_code: String,
    sample_topic: String,
}

fn parse_args() -> (PathBuf, usize, Option<u32>) {
    let mut dir: Option<PathBuf> = None;
    let mut max: usize = usize::MAX;
    let mut target_ledger: Option<u32> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => dir = args.next().map(PathBuf::from),
            "--max" => {
                max = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(usize::MAX)
            }
            "--target-ledger" => target_ledger = args.next().and_then(|s| s.parse().ok()),
            "-h" | "--help" => {
                eprintln!(
                    "usage: diag_overlap_check --dir <PARTITION_DIR> [--max N] [--target-ledger SEQ]"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }
    let dir = dir.expect("--dir is required");
    (dir, max, target_ledger)
}

fn ledger_metas(
    meta: &LedgerCloseMeta,
) -> Vec<(u32, [u8; 32], &TransactionResult, &TransactionMeta)> {
    let mut out = Vec::new();
    let (seq, processing): (u32, Vec<_>) = match meta {
        LedgerCloseMeta::V0(v) => (
            v.ledger_header.header.ledger_seq,
            v.tx_processing
                .iter()
                .map(|p| {
                    (
                        p.result.transaction_hash.0,
                        &p.result.result,
                        &p.tx_apply_processing,
                    )
                })
                .collect(),
        ),
        LedgerCloseMeta::V1(v) => (
            v.ledger_header.header.ledger_seq,
            v.tx_processing
                .iter()
                .map(|p| {
                    (
                        p.result.transaction_hash.0,
                        &p.result.result,
                        &p.tx_apply_processing,
                    )
                })
                .collect(),
        ),
        LedgerCloseMeta::V2(v) => (
            v.ledger_header.header.ledger_seq,
            v.tx_processing
                .iter()
                .map(|p| {
                    (
                        p.result.transaction_hash.0,
                        &p.result.result,
                        &p.tx_apply_processing,
                    )
                })
                .collect(),
        ),
    };
    for (hash, res, m) in processing {
        out.push((seq, hash, res, m));
    }
    out
}

fn tx_succeeded(result: &TransactionResult) -> bool {
    matches!(
        result.result,
        TransactionResultResult::TxSuccess(_) | TransactionResultResult::TxFeeBumpInnerSuccess(_)
    )
}

/// Render the first topic of the first orphan event for forensics.
/// Best-effort — falls back to "<unparsed>" if the topic shape is
/// unfamiliar.
fn first_topic_summary(v4: &TransactionMetaV4, per_op_set: &HashSet<Vec<u8>>) -> String {
    let limits = Limits::none();
    for diag in v4.diagnostic_events.iter() {
        if !matches!(diag.event.type_, ContractEventType::Contract) {
            continue;
        }
        let bytes = match diag.event.clone().to_xdr(limits.clone()) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if per_op_set.contains(&bytes) {
            continue;
        }
        // This is an orphan. Render its first topic.
        let ContractEventBody::V0(v0) = &diag.event.body;
        let mut parts = Vec::new();
        for (i, t) in v0.topics.iter().enumerate() {
            if i >= 3 {
                parts.push("...".to_string());
                break;
            }
            parts.push(format!("{:?}", t));
        }
        let in_success = diag.in_successful_contract_call;
        return format!("topics=[{}] in_succ_call={}", parts.join(", "), in_success);
    }
    "<no orphan>".into()
}

fn analyze_v4(
    seq: u32,
    tx_hash_hex: &str,
    v4: &TransactionMetaV4,
    result: &TransactionResult,
    stats: &mut Stats,
    target_hit: bool,
) {
    let successful = tx_succeeded(result);
    let result_code = format!("{:?}", result.result.discriminant());
    let limits = Limits::none();

    let mut per_op_set: HashSet<Vec<u8>> = HashSet::new();
    for op_meta in v4.operations.iter() {
        for ev in op_meta.events.iter() {
            if let Ok(bytes) = ev.clone().to_xdr(limits.clone()) {
                per_op_set.insert(bytes);
            }
        }
    }

    // Track Contract-typed entries in diag separately from the full diag
    // set so we can compute the orphan count (Contract entries in diag
    // with no match in per-op).
    let mut diag_set: HashSet<Vec<u8>> = HashSet::new();
    let mut diag_contract_set: HashSet<Vec<u8>> = HashSet::new();
    let mut diag_contract = 0usize;
    let mut diag_system = 0usize;
    let mut diag_diagnostic = 0usize;
    for diag in v4.diagnostic_events.iter() {
        let is_contract = matches!(diag.event.type_, ContractEventType::Contract);
        match diag.event.type_ {
            ContractEventType::Contract => diag_contract += 1,
            ContractEventType::System => diag_system += 1,
            ContractEventType::Diagnostic => diag_diagnostic += 1,
        }
        if let Ok(bytes) = diag.event.clone().to_xdr(limits.clone()) {
            if is_contract {
                diag_contract_set.insert(bytes.clone());
            }
            diag_set.insert(bytes);
        }
    }

    // Orphan = Contract-typed in diag whose bytes do NOT match any per-op
    // event. This is the data we'd actually lose by dropping the entire
    // diagnostic_events container.
    let orphan_contract: HashSet<&Vec<u8>> = diag_contract_set.difference(&per_op_set).collect();
    let orphan_count = orphan_contract.len();
    if !diag_set.is_empty() {
        if successful {
            stats.successful_tx_with_diag += 1;
        } else {
            stats.failed_tx_with_diag += 1;
        }
    }
    if orphan_count > 0 {
        stats.orphan_contract_in_diag += orphan_count;
        if successful {
            stats.orphan_from_successful_tx += orphan_count;
        } else {
            stats.orphan_from_failed_tx += orphan_count;
        }
        if stats.orphan_cases.len() < 20 {
            let sample_topic = first_topic_summary(v4, &per_op_set);
            stats.orphan_cases.push(OrphanCase {
                seq,
                tx_hash_hex: tx_hash_hex.to_string(),
                per_op_count: per_op_set.len(),
                diag_contract_count: diag_contract,
                orphan_count,
                tx_successful: successful,
                result_code: result_code.clone(),
                sample_topic,
            });
        }
    }

    let per_op_count = per_op_set.len();
    let diag_count = diag_set.len();
    let overlap: HashSet<&Vec<u8>> = per_op_set.intersection(&diag_set).collect();
    let overlap_count = overlap.len();

    stats.v4_txs += 1;
    stats.total_per_op_events += per_op_count;
    stats.total_diag_events += diag_count;
    stats.diag_inner_contract += diag_contract;
    stats.diag_inner_system += diag_system;
    stats.diag_inner_diagnostic += diag_diagnostic;
    if per_op_count > 0 {
        stats.v4_txs_with_per_op += 1;
    }
    if diag_count > 0 {
        stats.v4_txs_with_diag += 1;
    }
    if per_op_count > 0 && diag_count > 0 {
        stats.v4_txs_with_both += 1;
    }
    if overlap_count > 0 {
        stats.v4_txs_with_any_overlap += 1;
        stats.total_overlapping_pairs += overlap_count;
        if stats.sample_overlap_tx.is_none() {
            stats.sample_overlap_tx = Some(tx_hash_hex.to_string());
            stats.sample_per_op_count = per_op_count;
            stats.sample_diag_count = diag_count;
            stats.sample_overlap_count = overlap_count;
        }
    }

    if target_hit {
        // Tx-level events from v4.events
        let mut tx_level_contracts: Vec<String> = Vec::new();
        for tev in v4.events.iter() {
            if let Some(cid) = tev.event.contract_id.as_ref() {
                tx_level_contracts
                    .push(stellar_xdr::curr::ScAddress::Contract(cid.clone()).to_string());
            } else {
                tx_level_contracts.push("<no-contract>".into());
            }
        }
        let mut per_op_contracts: Vec<String> = Vec::new();
        for op in v4.operations.iter() {
            for ev in op.events.iter() {
                if let Some(cid) = ev.contract_id.as_ref() {
                    per_op_contracts
                        .push(stellar_xdr::curr::ScAddress::Contract(cid.clone()).to_string());
                } else {
                    per_op_contracts.push("<no-contract>".into());
                }
            }
        }

        stats.target_ledger_breakdown.push(TxBreakdown {
            tx_hash_hex: tx_hash_hex.to_string(),
            per_op_event_count: per_op_count,
            diag_event_count: diag_count,
            diag_contract,
            diag_system,
            diag_diagnostic,
            overlap_count,
            tx_level_event_count: v4.events.len(),
            tx_level_event_contracts: tx_level_contracts,
            per_op_event_contracts: per_op_contracts,
        });
    }
}

fn parse_seq_from_filename(name: &str) -> Option<u32> {
    // Galexie filename: {hex8}--{seq}[-{end}].xdr.zst
    let stem = name.strip_suffix(".xdr.zst")?;
    let (_, after_hex) = stem.split_once("--")?;
    let seq_str = after_hex
        .split_once('-')
        .map(|(s, _)| s)
        .unwrap_or(after_hex);
    seq_str.parse::<u32>().ok()
}

fn main() {
    let (dir, max, target_ledger) = parse_args();
    eprintln!(
        "[diag-overlap] dir={} max={} target_ledger={:?}",
        dir.display(),
        max,
        target_ledger
    );

    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("read_dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|n| n.ends_with(".xdr.zst"))
                .unwrap_or(false)
        })
        .collect();

    // Sort by sequence ascending so progress is monotonic.
    entries.sort_by_key(|p| {
        p.file_name()
            .and_then(|s| s.to_str())
            .and_then(parse_seq_from_filename)
            .unwrap_or(0)
    });

    if let Some(target) = target_ledger {
        // Always scan target first if specified, regardless of `max`.
        let mut reordered = Vec::new();
        let mut tail = Vec::new();
        for p in entries {
            if p.file_name()
                .and_then(|s| s.to_str())
                .and_then(parse_seq_from_filename)
                == Some(target)
            {
                reordered.push(p);
            } else {
                tail.push(p);
            }
        }
        reordered.extend(tail);
        entries = reordered;
    }

    let mut stats = Stats::default();
    let mut errors = 0usize;

    for (i, path) in entries.iter().enumerate() {
        if stats.ledgers_scanned >= max {
            break;
        }
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[warn] read {} failed: {e}", path.display());
                errors += 1;
                continue;
            }
        };
        let xdr = match decompress_zstd(&bytes) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[warn] decompress {} failed: {:?}", path.display(), e.kind);
                errors += 1;
                continue;
            }
        };
        let batch = match deserialize_batch(&xdr) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[warn] deserialize {} failed: {:?}", path.display(), e.kind);
                errors += 1;
                continue;
            }
        };

        for meta in batch.ledger_close_metas.iter() {
            let infos = ledger_metas(meta);
            let target_hit = target_ledger
                .map(|t| infos.first().map(|(s, _, _, _)| *s == t).unwrap_or(false))
                .unwrap_or(false);
            for (seq, hash, result, tx_meta) in infos {
                if let TransactionMeta::V4(v4) = tx_meta {
                    let hex = hex::encode(hash);
                    analyze_v4(seq, &hex, v4, result, &mut stats, target_hit);
                }
            }
        }
        stats.ledgers_scanned += 1;
        if (i + 1) % 50 == 0 {
            eprintln!(
                "[progress] scanned={} v4_txs={} v4_with_diag={} overlap_txs={}",
                stats.ledgers_scanned,
                stats.v4_txs,
                stats.v4_txs_with_diag,
                stats.v4_txs_with_any_overlap
            );
        }
    }

    println!();
    println!("=== Task 0182: V4 per-op ↔ diagnostic_events overlap report ===");
    println!(
        "ledgers scanned:                   {}",
        stats.ledgers_scanned
    );
    println!("decompress/parse errors:           {errors}");
    println!("V4 transactions:                   {}", stats.v4_txs);
    println!(
        "  with per-op events:              {}",
        stats.v4_txs_with_per_op
    );
    println!(
        "  with diagnostic events:          {}",
        stats.v4_txs_with_diag
    );
    println!(
        "  with both:                       {}",
        stats.v4_txs_with_both
    );
    println!(
        "  with ANY byte-overlap:           {}  ← mirror hypothesis predicts == 'with both'",
        stats.v4_txs_with_any_overlap
    );
    println!();
    println!("Aggregate event counts (deduplicated within each tx):");
    println!(
        "  per-op events:                   {}",
        stats.total_per_op_events
    );
    println!(
        "  diagnostic events:               {} ({} Contract / {} System / {} Diagnostic inner type)",
        stats.total_diag_events,
        stats.diag_inner_contract,
        stats.diag_inner_system,
        stats.diag_inner_diagnostic
    );
    println!(
        "  overlapping (per_op ∩ diag):     {}",
        stats.total_overlapping_pairs
    );
    if stats.total_per_op_events > 0 {
        let pct = (stats.total_overlapping_pairs as f64 / stats.total_per_op_events as f64) * 100.0;
        println!(
            "  overlap / per-op:                {:.2}%  ← mirror hypothesis predicts ~100%",
            pct
        );
    }

    if let Some(tx) = stats.sample_overlap_tx.as_ref() {
        println!();
        println!("Sample tx with overlap:");
        println!("  tx hash:                         {tx}");
        println!(
            "  per-op count:                    {}",
            stats.sample_per_op_count
        );
        println!(
            "  diag count:                      {}",
            stats.sample_diag_count
        );
        println!(
            "  overlap count:                   {}",
            stats.sample_overlap_count
        );
    } else if stats.v4_txs_with_both > 0 {
        println!();
        println!(
            "[ZERO byte-overlap across {} V4 txs that have both per-op and diag events]",
            stats.v4_txs_with_both
        );
        println!("Mirror hypothesis FALSIFIED on this sample.");
    }

    if !stats.target_ledger_breakdown.is_empty() {
        println!();
        println!("=== Target ledger breakdown ===");
        for tx in stats
            .target_ledger_breakdown
            .iter()
            .filter(|t| t.tx_level_event_count > 0 || t.per_op_event_count > 0)
        {
            println!(
                "  tx {} tx_level={} per_op={} diag={} (C={}/S={}/D={}) overlap={}",
                tx.tx_hash_hex,
                tx.tx_level_event_count,
                tx.per_op_event_count,
                tx.diag_event_count,
                tx.diag_contract,
                tx.diag_system,
                tx.diag_diagnostic,
                tx.overlap_count
            );
            if !tx.tx_level_event_contracts.is_empty() {
                println!(
                    "     tx_level contracts:  {:?}",
                    tx.tx_level_event_contracts
                );
            }
            if !tx.per_op_event_contracts.is_empty() {
                println!("     per_op   contracts:  {:?}", tx.per_op_event_contracts);
            }
        }
    }

    // ===================================================================
    // THE 1000% SAFETY CHECK
    // ===================================================================
    println!();
    println!("=== ORPHAN CHECK (the 1000% safety question) ===");
    println!("Contract-typed entries in v4.diagnostic_events that have NO match in per-op events:");
    println!(
        "  total Contract-typed in diag (across all V4 txs):  {}",
        stats.diag_inner_contract
    );
    println!(
        "  total overlapping pairs (per_op ∩ diag):           {}",
        stats.total_overlapping_pairs
    );
    println!(
        "  ORPHAN Contract-typed (would be LOST by drop):     {}",
        stats.orphan_contract_in_diag
    );
    if stats.orphan_contract_in_diag == 0 {
        println!();
        println!("  ✅ ZERO orphans. EVERY Contract-typed entry in diagnostic_events");
        println!("     has a byte-identical match in per-op events. Dropping the");
        println!("     diagnostic_events container loses no Contract events that are");
        println!("     not already kept elsewhere.");
    } else {
        println!();
        println!("  ⚠️  ORPHANS FOUND.");
        println!();
        println!("  Breakdown by tx success status:");
        println!(
            "     orphans from FAILED tx:     {}  (safe to drop — failed contract calls' \
             debug events, not consensus)",
            stats.orphan_from_failed_tx
        );
        println!(
            "     orphans from SUCCESSFUL tx: {}  ← CRITICAL if non-zero",
            stats.orphan_from_successful_tx
        );
        println!();
        println!("  V4 txs WITH diagnostic_events:");
        println!("     failed:     {}", stats.failed_tx_with_diag);
        println!("     successful: {}", stats.successful_tx_with_diag);
        println!();
        println!("  Sample orphan cases (up to 20):");
        for c in &stats.orphan_cases {
            println!(
                "     ledger {} tx {} per_op={} diag_C={} orphan={} success={} code={} | {}",
                c.seq,
                c.tx_hash_hex,
                c.per_op_count,
                c.diag_contract_count,
                c.orphan_count,
                c.tx_successful,
                c.result_code,
                c.sample_topic
            );
        }
    }

    println!();
    println!("=== MEASURED OUTCOME ===");
    if stats.total_overlapping_pairs == 0 && stats.v4_txs_with_both > 0 {
        println!(
            "Zero byte-overlap across {} V4 txs that have both per-op and diagnostic events.",
            stats.v4_txs_with_both
        );
        println!("diagnostic_events on this sample contains only host-side trace entries");
        println!("(fn_call / fn_return / core_metrics / errors), no Contract-typed copies.");
    } else if stats.total_overlapping_pairs > 0 {
        let pct = (stats.total_overlapping_pairs as f64 / stats.total_per_op_events as f64) * 100.0;
        println!(
            "{:.1}% of per-op events ({} of {}) appear byte-identically in diagnostic_events.",
            pct, stats.total_overlapping_pairs, stats.total_per_op_events
        );
        println!("Filtering by inner event_type would let those copies leak into the index;");
        println!("filtering by source container drops them in one step (task 0182 fix).");
    }
    println!();
    println!("Orphan count is the safety-critical metric: if `orphans from SUCCESSFUL tx`");
    println!("is non-zero, dropping diagnostic_events would lose consensus events that no");
    println!("other Stellar tool indexes. Run on every Galexie / stellar-host upgrade.");
}
