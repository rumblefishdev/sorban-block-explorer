//! Fixture-based regression for task 0181: every ledger parsed from a
//! real Galexie batch must surface the canonical hash already populated
//! by stellar-core in `LedgerHeaderHistoryEntry.hash`, not a
//! locally-recomputed `SHA256(header_xdr)` digest.
//!
//! Skipped cleanly when no fixture is present (CI). The unit tests in
//! `crates/xdr-parser/src/ledger.rs` cover the contract synthetically.

use std::path::PathBuf;

use stellar_xdr::curr::{LedgerCloseMeta, LedgerHeaderHistoryEntry};

#[test]
fn extract_ledger_returns_canonical_entry_hash_for_every_ledger_in_batch() {
    let Some(path) = locate_fixture() else {
        eprintln!(
            "no XDR fixture available (set XDR_FIXTURE or place .xdr.zst under \
             .temp/) — skipping ledger-hash canonicality check"
        );
        return;
    };

    let bytes = std::fs::read(&path).expect("read fixture");
    let decompressed = xdr_parser::decompress_zstd(&bytes).expect("zstd decode");
    let batch = xdr_parser::deserialize_batch(&decompressed).expect("xdr decode");

    let mut checked = 0usize;
    for meta in batch.ledger_close_metas.iter() {
        let entry = header_entry(meta);
        let extracted = xdr_parser::extract_ledger(meta);

        assert_eq!(
            extracted.hash,
            hex::encode(entry.hash.0),
            "ledger {}: parser hash diverged from canonical entry.hash.0 — \
             this is the bug 0181 fix",
            entry.header.ledger_seq,
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "fixture batch contained no ledgers — test would silently pass. \
         fixture path may be wrong."
    );
    eprintln!("ledger.hash verified canonical for {checked} ledgers");
}

fn header_entry(meta: &LedgerCloseMeta) -> &LedgerHeaderHistoryEntry {
    match meta {
        LedgerCloseMeta::V0(v) => &v.ledger_header,
        LedgerCloseMeta::V1(v) => &v.ledger_header,
        LedgerCloseMeta::V2(v) => &v.ledger_header,
    }
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
        .join("../../.temp/FC4DB5FF--62016000-62079999/FC4DB59C--62016099.xdr.zst");
    p.is_file().then_some(p)
}
