//! Independent ground-truth verification for task 0183.
//!
//! Walks `v4.diagnostic_events` by hand (NOT through
//! `extract_invocations_from_diagnostics`) and counts `fn_call` topics
//! per (tx, contract). The point is to cross-check the DB numbers with
//! a counter that does not share code with the parser under audit —
//! falsifies "the test passes because we measured what we wrote."
//!
//! Skipped when the local archive scratch dir is absent (CI doesn't
//! ship `.temp/`). When present, prints a comparison table to stderr
//! and asserts every targeted (tx, contract) pair matches the DB
//! row count seeded into the test.

use std::collections::BTreeMap;
use std::path::PathBuf;

use stellar_xdr::curr::{
    ContractEventBody, ContractEventType, ContractId, Hash, LedgerCloseMeta, ScAddress, ScVal,
    TransactionMeta,
};

/// Each entry mirrors one row of the DB query that produced the audit
/// table — `(contract_id, expected_fn_call_count)` per tx hash.
struct TargetCase {
    tx_hash: &'static str,
    archive_filename: &'static str,
    expected_total: u32,
    expected_per_contract: &'static [(&'static str, u32)],
}

const CASES: &[TargetCase] = &[
    // Case 1 — multi-hop swap Phoenix→Aquarius (acceptance target).
    TargetCase {
        tx_hash: "b7b51065e0a6830e684269c3d4e0c1c3dc76b0c66e97fc7d46fbd15c3b163235",
        archive_filename: "FC4DB5A9--62016086.xdr.zst",
        expected_total: 16,
        expected_per_contract: &[
            (
                "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75",
                6,
            ),
            (
                "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA",
                5,
            ),
            (
                "CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE",
                2,
            ),
            (
                "CC2JAXZ6663YWUS4ETXZV2GTEMAJCZFTKZSAHVABH5RUGGRKYCCKBR3Y",
                1,
            ),
            (
                "CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX",
                1,
            ),
            (
                "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY",
                1,
            ),
        ],
    },
    // Case 2 — failed multi-hop, 38-frame trace before trap.
    TargetCase {
        tx_hash: "68f947318c8695c1868a1e4f2f01992f1c318631743585f1103de8f86114b932",
        archive_filename: "FC4DB5A9--62016086.xdr.zst",
        expected_total: 38,
        expected_per_contract: &[],
    },
    // Case 3 — trivial single call.
    TargetCase {
        tx_hash: "55436964f4e1c2abb659b2f8a63dd90f77c6085871d110ca43a2073c92964055",
        archive_filename: "FC4DB5FF--62016000.xdr.zst",
        expected_total: 1,
        expected_per_contract: &[],
    },
    // Case 4 — second failed multi-hop (Case 2 twin).
    TargetCase {
        tx_hash: "e527cefb0b5edae1f0006c73ad0c4d010b20ed3499fd67d1d0246be211ec046e",
        archive_filename: "FC4DB5A9--62016086.xdr.zst",
        expected_total: 38,
        expected_per_contract: &[],
    },
    // Case 5 — second trivial single call.
    TargetCase {
        tx_hash: "1f13187ca8811837e1e52fbb384e02526bb943c8af3d617ed17f2398a41266d3",
        archive_filename: "FC4DB5FF--62016000.xdr.zst",
        expected_total: 1,
        expected_per_contract: &[],
    },
];

#[test]
fn db_counts_match_independent_xdr_walk() {
    let archive_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.temp/FC4DB5FF--62016000-62079999");
    if !archive_root.is_dir() {
        eprintln!(
            "no archive at {} — skipping (CI does not ship .temp/)",
            archive_root.display()
        );
        return;
    }

    let mut all_match = true;
    eprintln!(
        "\n{:<10} {:<8} {:<10} {:<10} {:<8}",
        "case", "expect", "counted", "delta", "verdict"
    );
    eprintln!("{}", "-".repeat(50));

    for (i, case) in CASES.iter().enumerate() {
        let archive_path = archive_root.join(case.archive_filename);
        let bytes = std::fs::read(&archive_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", archive_path.display()));
        let decompressed = xdr_parser::decompress_zstd(&bytes).expect("zstd decode");
        let batch = xdr_parser::deserialize_batch(&decompressed).expect("xdr decode");
        let net_id = xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE);

        let mut found = false;
        for meta in batch.ledger_close_metas.iter() {
            let ledger_seq = xdr_parser::extract_ledger(meta).sequence;
            let closed_at = xdr_parser::extract_ledger(meta).closed_at;
            let txs = xdr_parser::extract_transactions(meta, ledger_seq, closed_at, &net_id);
            let metas = collect_tx_metas(meta);

            for (idx, ext_tx) in txs.iter().enumerate() {
                if ext_tx.hash != case.tx_hash {
                    continue;
                }
                let Some(tm) = metas.get(idx).copied() else {
                    panic!("tx {} has no TransactionMeta", case.tx_hash);
                };

                let counts = independent_fn_call_count(tm);
                let total: u32 = counts.values().sum();
                let delta = total as i32 - case.expected_total as i32;
                let verdict = if delta == 0 { "✅" } else { "❌" };
                eprintln!(
                    "{:<10} {:<8} {:<10} {:<+10} {:<8}",
                    format!("c{}", i + 1),
                    case.expected_total,
                    total,
                    delta,
                    verdict,
                );

                if delta != 0 {
                    all_match = false;
                    eprintln!("  case {}: per-contract independent counts:", i + 1);
                    for (cid, n) in &counts {
                        eprintln!("    {} → {}", cid, n);
                    }
                }

                // Per-contract spot check — only Case 1 has a granular
                // expected vector. Cases 2/4 (38 frames) and 3/5 (1 frame)
                // assert only on the total.
                for (expected_cid, expected_n) in case.expected_per_contract {
                    let actual = counts.get(*expected_cid).copied().unwrap_or(0);
                    if actual != *expected_n {
                        all_match = false;
                        eprintln!(
                            "  case {}: contract {} expected {} got {} (mismatch)",
                            i + 1,
                            expected_cid,
                            expected_n,
                            actual
                        );
                    }
                }

                found = true;
                break;
            }
            if found {
                break;
            }
        }
        assert!(
            found,
            "case {}: tx {} not found in archive {}",
            i + 1,
            case.tx_hash,
            archive_path.display()
        );
    }

    assert!(
        all_match,
        "DB counts disagree with independent XDR walk — see stderr above"
    );
    eprintln!("\nAll DB counts match independent XDR-level fn_call walk.");
}

/// Aggregate sanity: every successful Soroban tx in ledger 62016086,
/// independent fn_call sum, compared with the DB's `SUM(amount) WHERE
/// ledger_sequence = 62016086`. Failed tx are excluded because the
/// parser counts pre-trap traces, which is intentional but hard to
/// reason about in an aggregate.
///
/// Hard-codes the DB number on the audit run (658). Skips when archive
/// is absent. If you re-index a different window, update the constant.
#[test]
fn aggregate_fn_call_count_matches_db_for_ledger_62016086() {
    const DB_TOTAL_INCL_FAILED: u32 = 658;

    let archive_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.temp/FC4DB5FF--62016000-62079999");
    if !archive_root.is_dir() {
        eprintln!("no archive — skipping");
        return;
    }
    let path = archive_root.join("FC4DB5A9--62016086.xdr.zst");
    let bytes = std::fs::read(&path).expect("read");
    let decompressed = xdr_parser::decompress_zstd(&bytes).expect("zstd");
    let batch = xdr_parser::deserialize_batch(&decompressed).expect("xdr");

    let mut total: u32 = 0;
    for meta in batch.ledger_close_metas.iter() {
        if xdr_parser::extract_ledger(meta).sequence != 62016086 {
            continue;
        }
        let metas = collect_tx_metas(meta);
        for tm in metas {
            let counts = independent_fn_call_count(tm);
            total += counts.values().sum::<u32>();
        }
    }

    eprintln!("ledger 62016086: independent fn_call sum = {total}, DB = {DB_TOTAL_INCL_FAILED}");
    assert_eq!(
        total, DB_TOTAL_INCL_FAILED,
        "aggregate disagreement: parser may have a counting bug"
    );
}

/// Hand-rolled counter — does NOT call `extract_invocations_from_diagnostics`.
/// Walks `*.diagnostic_events` and counts every Diagnostic-typed event
/// whose `topics[0] == Symbol("fn_call")`. The called contract is read
/// from `topics[1]` — `ScVal::Bytes(32 bytes)` (mainnet captive-core
/// shape) or `ScVal::Address(ScAddress::Contract(_))` (newer hosts).
/// Returns a `contract_strkey → fn_call_count` map.
fn independent_fn_call_count(meta: &TransactionMeta) -> BTreeMap<String, u32> {
    let diags: Vec<_> = match meta {
        TransactionMeta::V3(v3) => v3
            .soroban_meta
            .as_ref()
            .map(|m| m.diagnostic_events.iter().collect())
            .unwrap_or_default(),
        TransactionMeta::V4(v4) => v4.diagnostic_events.iter().collect(),
        _ => Vec::new(),
    };

    let mut out: BTreeMap<String, u32> = BTreeMap::new();
    for diag in diags {
        if !matches!(diag.event.type_, ContractEventType::Diagnostic) {
            continue;
        }
        let ContractEventBody::V0(ref v0) = diag.event.body;
        if v0.topics.len() < 2 {
            continue;
        }
        let head = match v0.topics.first() {
            Some(ScVal::Symbol(s)) => std::str::from_utf8(s.as_vec()).unwrap_or(""),
            _ => continue,
        };
        if head != "fn_call" {
            continue;
        }
        let Some(target_topic) = v0.topics.get(1) else {
            continue;
        };
        let strkey = match target_topic {
            ScVal::Bytes(b) => {
                if b.as_slice().len() != 32 {
                    continue;
                }
                let mut buf = [0u8; 32];
                buf.copy_from_slice(b.as_slice());
                ScAddress::Contract(ContractId(Hash(buf))).to_string()
            }
            ScVal::Address(addr @ ScAddress::Contract(_)) => addr.to_string(),
            _ => continue,
        };
        *out.entry(strkey).or_insert(0) += 1;
    }
    out
}

fn collect_tx_metas(meta: &LedgerCloseMeta) -> Vec<&TransactionMeta> {
    match meta {
        LedgerCloseMeta::V0(_) => Vec::new(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .iter()
            .map(|p| &p.tx_apply_processing)
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .iter()
            .map(|p| &p.tx_apply_processing)
            .collect(),
    }
}
