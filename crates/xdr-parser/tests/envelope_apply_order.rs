//! Integration test for tasks 0168/0169/0170: verifies
//! `xdr_parser::envelope::extract_envelopes` returns envelopes aligned 1:1
//! with `tx_processing` (apply order), matched by `transaction_hash`.
//!
//! Loads a real mainnet ledger from `.temp/` if present. Skipped in CI
//! (where the archive isn't shipped) and gated on the `XDR_FIXTURE` env
//! var pointing at a `.xdr.zst` Galexie batch.

use std::path::PathBuf;

use stellar_xdr::curr::LedgerCloseMeta;

#[test]
fn extract_envelopes_aligns_with_tx_processing() {
    let path = locate_fixture();
    let Some(path) = path else {
        eprintln!(
            "no XDR fixture available (set XDR_FIXTURE or place .xdr.zst under \
             .temp/) — skipping alignment test"
        );
        return;
    };

    let bytes = std::fs::read(&path).expect("read fixture");
    let decompressed = xdr_parser::decompress_zstd(&bytes).expect("zstd decode");
    let batch = xdr_parser::deserialize_batch(&decompressed).expect("xdr decode");

    let net_id = xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE);

    for meta in batch.ledger_close_metas.iter() {
        let envelopes = xdr_parser::envelope::extract_envelopes(meta, &net_id);
        let processing = tx_processing_hashes(meta);

        assert_eq!(
            envelopes.len(),
            processing.len(),
            "envelopes vec must align 1:1 with tx_processing"
        );

        for (i, (env_opt, expected_hash)) in envelopes.iter().zip(processing.iter()).enumerate() {
            let env = env_opt.as_ref().unwrap_or_else(|| {
                panic!(
                    "slot {i}: no envelope matched tx_processing hash {} \
                     (expected for every well-formed LedgerCloseMeta)",
                    hex::encode(expected_hash)
                )
            });
            let computed = xdr_parser::envelope::tx_envelope_hash(env, &net_id);
            assert_eq!(
                &computed, expected_hash,
                "slot {i}: envelope hash mismatch — alignment broken"
            );
        }
    }
}

fn locate_fixture() -> Option<PathBuf> {
    // If `XDR_FIXTURE` is set, treat it as authoritative — a misconfigured
    // path must fail loudly rather than silently fall through to the
    // implicit `.temp/` scratch dir, which would otherwise look like a
    // passing run.
    if let Ok(p) = std::env::var("XDR_FIXTURE") {
        let pb = PathBuf::from(&p);
        assert!(
            pb.is_file(),
            "XDR_FIXTURE is set but does not point to an existing file: {p}"
        );
        return Some(pb);
    }
    // Fallback: the audit ledger 62016099 if the local archive scratch
    // dir happens to be present. Skipping is fine here — the synthetic
    // unit tests in `crates/xdr-parser/src/envelope.rs` already cover
    // the alignment logic without a real fixture.
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.temp/FC4DB5FF--62016000-62079999/FC4DB59C--62016099.xdr.zst");
    p.is_file().then_some(p)
}

/// Audit-ledger regression for task 0169 / Finding 1.
///
/// stellar-core records the canonical inner-transaction hash inside
/// `LedgerCloseMeta` itself: the per-tx `TransactionResult` carries
/// `TxFeeBumpInnerSuccess(InnerTransactionResultPair)` /
/// `TxFeeBumpInnerFailed(...)` whose `transaction_hash` field is, per
/// the XDR comment, "hash of the inner transaction". This is the value
/// every validator computes during consensus and the source from which
/// Horizon derives `inner_transaction.hash`.
///
/// We assert that for **every** fee-bump tx in the audit ledger the
/// helper `xdr_parser::envelope::inner_tx_hash` produces the same bytes
/// stellar-core itself wrote into `InnerTransactionResultPair`. Since
/// the comparison is against the consensus-recorded value (not via any
/// external service), passing this test means the helper is
/// byte-for-byte equivalent to stellar-core's inner-tx hashing under
/// the mainnet network_id.
///
/// Skipped if the fixture is absent. The synthetic unit tests in
/// `envelope.rs` cover the algorithm without a fixture.
#[test]
fn inner_tx_hash_matches_stellar_core_for_every_fee_bump_in_audit_ledger() {
    use stellar_xdr::curr::TransactionResultResult;
    use xdr_parser::envelope::{inner_tx_hash, tx_envelope_hash};

    let Some(path) = locate_fixture() else {
        eprintln!("no XDR fixture available — skipping audit-ledger inner_tx_hash check");
        return;
    };

    let bytes = std::fs::read(&path).expect("read fixture");
    let decompressed = xdr_parser::decompress_zstd(&bytes).expect("zstd decode");
    let batch = xdr_parser::deserialize_batch(&decompressed).expect("xdr decode");
    let net_id = xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE);

    let mut fee_bumps_checked = 0usize;
    for meta in batch.ledger_close_metas.iter() {
        let envelopes = xdr_parser::envelope::extract_envelopes(meta, &net_id);
        let processing = tx_processing_results(meta);
        assert_eq!(envelopes.len(), processing.len(), "alignment broken");

        for (i, (env_opt, result)) in envelopes.iter().zip(processing.iter()).enumerate() {
            let Some(env) = env_opt else { continue };
            // Only fee-bump txs carry an InnerTransactionResultPair.
            let core_inner_hash = match &result.result {
                TransactionResultResult::TxFeeBumpInnerSuccess(pair)
                | TransactionResultResult::TxFeeBumpInnerFailed(pair) => pair.transaction_hash.0,
                _ => continue,
            };

            let our_inner_hash = inner_tx_hash(env, &net_id).unwrap_or_else(|| {
                panic!(
                    "slot {i}: tx_processing carries InnerTransactionResultPair \
                     (fee-bump), but inner_tx_hash returned None for the matched \
                     envelope. outer_hash={}",
                    hex::encode(tx_envelope_hash(env, &net_id))
                )
            });
            assert_eq!(
                our_inner_hash,
                core_inner_hash,
                "slot {i}: our inner_tx_hash != stellar-core's recorded \
                 InnerTransactionResultPair.transaction_hash. \
                 outer={}, ours={}, core={}",
                hex::encode(tx_envelope_hash(env, &net_id)),
                hex::encode(our_inner_hash),
                hex::encode(core_inner_hash),
            );
            fee_bumps_checked += 1;
        }
    }

    assert!(
        fee_bumps_checked > 0,
        "no fee-bump txs found in fixture — test would silently pass \
         without exercising the helper. fixture path may be wrong."
    );
    eprintln!("inner_tx_hash verified against stellar-core meta for {fee_bumps_checked} fee-bumps");
}

fn tx_processing_results(meta: &LedgerCloseMeta) -> Vec<&stellar_xdr::curr::TransactionResult> {
    match meta {
        LedgerCloseMeta::V0(v) => v.tx_processing.iter().map(|p| &p.result.result).collect(),
        LedgerCloseMeta::V1(v) => v.tx_processing.iter().map(|p| &p.result.result).collect(),
        LedgerCloseMeta::V2(v) => v.tx_processing.iter().map(|p| &p.result.result).collect(),
    }
}

fn tx_processing_hashes(meta: &LedgerCloseMeta) -> Vec<[u8; 32]> {
    match meta {
        LedgerCloseMeta::V0(v) => v
            .tx_processing
            .iter()
            .map(|p| p.result.transaction_hash.0)
            .collect(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .iter()
            .map(|p| p.result.transaction_hash.0)
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .iter()
            .map(|p| p.result.transaction_hash.0)
            .collect(),
    }
}
