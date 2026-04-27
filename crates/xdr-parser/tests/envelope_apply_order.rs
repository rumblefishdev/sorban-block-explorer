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
