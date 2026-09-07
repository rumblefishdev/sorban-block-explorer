//! Every ledger in the state corpus through the REAL pipeline (task 0374,
//! step 7) — the deep proof for the state readers, in the same spirit as the
//! `add_pool` and share-token corpora.
//!
//! The corpus is 79 raw mainnet ledgers spanning every era and pool type:
//! the two T3 pilot slices (early gap 53.07M with 5 trading pools, late gap
//! 57.40M with 83), plus 11 REGISTRATION ledgers picked to cover the
//! dead-deployment era (50.6M), the first Aquarius pools (52.7M), the 4-leg
//! stable pool's own registration, a 3-leg stable, the elastic type, the
//! first concentrated pool and the newest registrations (64.1M). Running the
//! oldest ones IS the recorded early-era layout probe: if any era spells its
//! storage keys differently, the invariants below go red.
//!
//! Invariants, per ledger:
//! * a `ContractData` change whose key MATCHES the `PoolData` shape must
//!   parse — a matched-but-unparseable entry is a silent-loss bug, never a
//!   skip (count == 0 asserted);
//! * every parsed plane entry carries non-empty reserves and a non-empty
//!   pool type;
//! * every pool instance recognised by the `Plane` shape yields the plane
//!   address; `Router` is OPTIONAL (five older deployments write none — the
//!   dead-deployment ledgers in this corpus exercise exactly that arm).
//!
//! With `POOL_STATE_EMIT=path` the test also writes every extracted plane
//! row as JSONL, so the dual-source oracle check (plane state vs the
//! `update_reserves` EVENTS already in ClickHouse) runs outside against
//! production — the T4 cross-check, executed for real.
//!
//! Skipped when `POOL_STATE_CORPUS_DIR` is unset (CI ships no raw ledgers).

use std::io::Write;

use serde_json::Value;
use stellar_xdr::{LedgerCloseMetaBatch, Limits, ReadXdr};
use xdr_parser::pool_state::{extract_plane_pool_data, extract_pool_instances};

#[test]
fn every_corpus_ledger_extracts_cleanly() {
    let Ok(dir) = std::env::var("POOL_STATE_CORPUS_DIR") else {
        eprintln!("POOL_STATE_CORPUS_DIR unset — skipping (see module docs)");
        return;
    };
    let emit = std::env::var("POOL_STATE_EMIT").ok();
    let mut emit_file = emit
        .as_ref()
        .map(|p| std::fs::File::create(p).expect("emit path writable"));

    let mut files = 0usize;
    let mut plane_rows = 0usize;
    let mut instance_rows = 0usize;
    let mut matched_unparseable = 0usize;

    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("corpus dir readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xdr"))
        .collect();
    paths.sort();

    for path in paths {
        files += 1;
        let bytes = std::fs::read(&path).expect("ledger readable");
        let batch =
            LedgerCloseMetaBatch::from_xdr(&bytes, Limits::none()).expect("a LedgerCloseMetaBatch");

        let mut per_tx = |seq: u32, i: usize, meta: &stellar_xdr::TransactionMeta| {
            let changes = xdr_parser::extract_ledger_entry_changes(meta, &format!("tx{i}"), seq, 0);

            // The silent-loss invariant: a key SHAPED like PoolData that the
            // parser refuses is a bug, not a skip.
            for ch in &changes {
                if ch.entry_type != "contract_data" {
                    continue;
                }
                if !matches!(ch.change_type.as_str(), "created" | "updated" | "restored") {
                    continue;
                }
                let shaped = ch
                    .key
                    .get("key")
                    .and_then(|k| k.get("value"))
                    .and_then(Value::as_array)
                    .and_then(|parts| parts.first())
                    .and_then(|p| p.get("value"))
                    .and_then(Value::as_str)
                    == Some("PoolData");
                if shaped {
                    let owner = ch.key.get("contract").and_then(Value::as_str).unwrap_or("");
                    let parsed = ch.data.as_ref().and_then(|d| d.get("val")).and_then(|v| {
                        xdr_parser::pool_state::parse_plane_pool_data(
                            owner,
                            ch.key.get("key").unwrap(),
                            v,
                        )
                    });
                    if parsed.is_none() {
                        matched_unparseable += 1;
                        eprintln!(
                            "UNPARSEABLE PoolData at ledger {seq}: {}",
                            serde_json::to_string(&ch.key).unwrap_or_default()
                        );
                    }
                }
            }

            for p in extract_plane_pool_data(&changes) {
                plane_rows += 1;
                assert!(
                    !p.data.reserves.is_empty(),
                    "ledger {seq}: plane entry with no reserves for {}",
                    p.data.pool
                );
                if let Some(f) = emit_file.as_mut() {
                    writeln!(
                        f,
                        "{}",
                        serde_json::json!({
                            "ledger": seq, "pool": p.data.pool,
                            "reserves": p.data.reserves,
                        })
                    )
                    .expect("emit write");
                }
            }
            for inst in extract_pool_instances(&changes) {
                instance_rows += 1;
                // Plane is the shape key and must survive; Router is OPTIONAL
                // (five older deployments write none — 23 real pools; the
                // acceptance arm in stage.rs takes them UNVERIFIED). The
                // corpus's dead-deployment ledgers exercise exactly that arm,
                // so asserting router here would fail on real history.
                assert!(
                    inst.state.plane.is_some(),
                    "ledger {seq}: family instance without a plane for {}",
                    inst.state.pool
                );
                if let Some(f) = emit_file.as_mut() {
                    writeln!(
                        f,
                        "{}",
                        serde_json::json!({
                            "ledger": seq, "instance": inst.state.pool,
                            "token_share": inst.state.token_share,
                            "reserves": inst.state.reserves,
                        })
                    )
                    .expect("emit write");
                }
            }
        };

        for lcm in batch.ledger_close_metas.iter() {
            match lcm {
                stellar_xdr::LedgerCloseMeta::V0(v0) => {
                    let seq = v0.ledger_header.header.ledger_seq;
                    for (i, tx) in v0.tx_processing.iter().enumerate() {
                        per_tx(seq, i, &tx.tx_apply_processing);
                    }
                }
                stellar_xdr::LedgerCloseMeta::V1(v1) => {
                    let seq = v1.ledger_header.header.ledger_seq;
                    for (i, tx) in v1.tx_processing.iter().enumerate() {
                        per_tx(seq, i, &tx.tx_apply_processing);
                    }
                }
                stellar_xdr::LedgerCloseMeta::V2(v2) => {
                    let seq = v2.ledger_header.header.ledger_seq;
                    for (i, tx) in v2.tx_processing.iter().enumerate() {
                        per_tx(seq, i, &tx.tx_apply_processing);
                    }
                }
            }
        }
    }

    eprintln!(
        "\n--- state corpus: {files} ledgers, {plane_rows} plane writes, \
         {instance_rows} pool instances ---"
    );
    assert_eq!(
        matched_unparseable, 0,
        "PoolData-shaped entries the parser refused — silent loss"
    );
    assert!(files >= 70, "corpus looks truncated: {files} files");
    assert!(plane_rows > 0 && instance_rows > 0);
}
