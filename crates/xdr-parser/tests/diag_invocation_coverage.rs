//! Task 0183 — fixture-backed coverage check for the diagnostic-event
//! invocation walker.
//!
//! Targets a known multi-hop swap (Phoenix → Aquarius) that the auth tree
//! cannot represent: the user signs only the outer router call, all nested
//! pool calls execute under contract authority. stellar.expert renders ~12
//! contract-level invocations; the local DB previously had zero rows for
//! this tx (see task body for the E03 measurement). With the diag-tree
//! walker active, the parser must produce the full execution chain.
//!
//! Skipped (with a stderr breadcrumb) when the local archive scratch dir
//! is absent — CI does not ship `.temp/`. The synthetic walker tests in
//! `crates/xdr-parser/src/invocation.rs` cover the parser logic without a
//! fixture; this test is the end-to-end correctness gate against real
//! captive-core diagnostic output.

use std::path::PathBuf;

use stellar_xdr::curr::{LedgerCloseMeta, TransactionMeta};

const TARGET_TX: &str = "b7b51065e0a6830e684269c3d4e0c1c3dc76b0c66e97fc7d46fbd15c3b163235";
/// Manually verified against stellar.expert: the tx walks the router
/// (Phoenix multi-hop) into multiple Aquarius / Phoenix pools and back.
/// Lower-bound assertion — exact node count is host-revision sensitive
/// and not the hill we want to die on.
const MIN_EXPECTED_INVOCATIONS: usize = 8;

#[test]
fn diag_walker_recovers_multi_hop_swap_invocations() {
    let Some(path) = locate_fixture() else {
        eprintln!(
            "no XDR fixture available (set XDR_FIXTURE or place \
             FC4DB5A9--62016086.xdr.zst under .temp/) — skipping"
        );
        return;
    };

    let bytes = std::fs::read(&path).expect("read fixture");
    let decompressed = xdr_parser::decompress_zstd(&bytes).expect("zstd decode");
    let batch = xdr_parser::deserialize_batch(&decompressed).expect("xdr decode");

    let net_id = xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE);

    for meta in batch.ledger_close_metas.iter() {
        let txs =
            xdr_parser::extract_transactions(meta, ledger_seq(meta), closed_at(meta), &net_id);
        let metas = collect_tx_metas(meta);

        for (idx, ext_tx) in txs.iter().enumerate() {
            if ext_tx.hash != TARGET_TX {
                continue;
            }
            let Some(tm) = metas.get(idx).copied() else {
                panic!("target tx {TARGET_TX} has no TransactionMeta in tx_processing");
            };

            // Diag walker first: full execution coverage.
            let diag_invs = xdr_parser::extract_invocations_from_diagnostics(
                tm,
                &ext_tx.hash,
                ledger_seq(meta),
                closed_at(meta),
                &ext_tx.source_account,
                ext_tx.successful,
            );
            assert!(
                diag_invs.len() >= MIN_EXPECTED_INVOCATIONS,
                "diag walker produced {} rows for multi-hop swap, expected >= {}. \
                 Auth-less router sub-calls regressed?",
                diag_invs.len(),
                MIN_EXPECTED_INVOCATIONS,
            );

            // The execution chain must surface contract-to-contract
            // callers; otherwise we're effectively still on the auth
            // tree (which would have produced 0 rows for this tx).
            let contract_callers = diag_invs
                .iter()
                .filter(|inv| {
                    inv.caller_account
                        .as_deref()
                        .is_some_and(|s| s.starts_with('C'))
                })
                .count();
            assert!(
                contract_callers > 0,
                "every inv carries a G-account caller — diag walker is producing \
                 auth-tree-shaped output, not the execution tree"
            );

            // Every inv has a contract_id (no create-contract host fns
            // expected in a swap).
            assert!(
                diag_invs.iter().all(|i| i.contract_id.is_some()),
                "swap tx must produce contract-scoped invocations only"
            );

            eprintln!(
                "tx {TARGET_TX}: diag walker produced {} invocations \
                 ({} contract→contract callers)",
                diag_invs.len(),
                contract_callers,
            );
            return;
        }
    }
    panic!(
        "target tx {TARGET_TX} not found in fixture {}",
        path.display()
    );
}

fn locate_fixture() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("XDR_FIXTURE") {
        let pb = PathBuf::from(&p);
        assert!(
            pb.is_file(),
            "XDR_FIXTURE is set but does not point to an existing file: {p}"
        );
        return Some(pb);
    }
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.temp/FC4DB5FF--62016000-62079999/FC4DB5A9--62016086.xdr.zst");
    p.is_file().then_some(p)
}

fn ledger_seq(meta: &LedgerCloseMeta) -> u32 {
    xdr_parser::extract_ledger(meta).sequence
}

fn closed_at(meta: &LedgerCloseMeta) -> i64 {
    xdr_parser::extract_ledger(meta).closed_at
}

/// Mirrors `indexer::handler::process::collect_tx_metas` — pull the
/// per-tx `TransactionMeta` aligned 1:1 with `tx_processing`. Inlined
/// here to avoid pulling indexer as a test-only dep on this crate.
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
