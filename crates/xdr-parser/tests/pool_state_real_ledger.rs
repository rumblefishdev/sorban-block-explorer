//! The registration ledger, end to end through the REAL pipeline
//! (task 0374, step 7).
//!
//! Unit fixtures pin hand-translated payloads; this test feeds the raw
//! mainnet ledger 63 893 403 (whose `add_pool` registered pool `CBMWU357…`)
//! through `extract_ledger_entry_changes` — the exact code the indexer runs —
//! and then through both state extractors. It exists because two dialect
//! traps were caught building this step (CLI-vs-house typed JSON, and
//! `scval_to_typed_json` silently DROPPING instance storage); only a
//! raw-ledger run proves the whole chain of custody at once.
//!
//! Skipped (passes trivially) when `POOL_STATE_LEDGER` is unset. Harvest:
//!
//! ```sh
//! # bucket path math in crates/backfill-runner/src/partition.rs
//! curl -s https://aws-public-blockchain.s3.amazonaws.com/v1.1/stellar/ledgers/pubnet/\
//! FC3125FF--63872000-63935999/FC3155D4--63893403.xdr.zst | zstd -d > /tmp/reg63893403.xdr
//! POOL_STATE_LEDGER=/tmp/reg63893403.xdr \
//!   cargo test -p xdr-parser --test pool_state_real_ledger -- --nocapture
//! ```

use stellar_xdr::{LedgerCloseMetaBatch, Limits, ReadXdr};
use xdr_parser::pool_state::{
    ExtractedPlanePoolData, ExtractedPoolInstance, extract_plane_pool_data, extract_pool_instances,
};

const POOL: &str = "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH";
const PLANE: &str = "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY";
const SHARE: &str = "CC5PU23MKXHUFJKGG5FAUG7MFZX2KMWXPNZP26DDYW76VCB26UWMPEI6";

#[test]
fn the_registration_ledger_yields_state_for_the_new_pool() {
    let Ok(path) = std::env::var("POOL_STATE_LEDGER") else {
        eprintln!("POOL_STATE_LEDGER unset — skipping (see module docs)");
        return;
    };
    let bytes = std::fs::read(&path).expect("ledger file readable");
    let batch =
        LedgerCloseMetaBatch::from_xdr(&bytes, Limits::none()).expect("a LedgerCloseMetaBatch");

    let mut planes: Vec<ExtractedPlanePoolData> = Vec::new();
    let mut instances: Vec<ExtractedPoolInstance> = Vec::new();
    {
        let mut process = |seq: u32, i: usize, meta: &stellar_xdr::TransactionMeta| {
            let changes = xdr_parser::extract_ledger_entry_changes(meta, &format!("tx{i}"), seq, 0);
            planes.extend(extract_plane_pool_data(&changes));
            instances.extend(extract_pool_instances(&changes));
        };
        for lcm in batch.ledger_close_metas.iter() {
            match lcm {
                stellar_xdr::LedgerCloseMeta::V0(v0) => {
                    let seq = v0.ledger_header.header.ledger_seq;
                    for (i, tx) in v0.tx_processing.iter().enumerate() {
                        process(seq, i, &tx.tx_apply_processing);
                    }
                }
                stellar_xdr::LedgerCloseMeta::V1(v1) => {
                    let seq = v1.ledger_header.header.ledger_seq;
                    for (i, tx) in v1.tx_processing.iter().enumerate() {
                        process(seq, i, &tx.tx_apply_processing);
                    }
                }
                stellar_xdr::LedgerCloseMeta::V2(v2) => {
                    let seq = v2.ledger_header.header.ledger_seq;
                    for (i, tx) in v2.tx_processing.iter().enumerate() {
                        process(seq, i, &tx.tx_apply_processing);
                    }
                }
            }
        }
    }

    eprintln!(
        "\n--- ledger 63 893 403: {} plane writes, {} pool instances ---",
        planes.len(),
        instances.len()
    );

    // The new pool's plane entry, exactly as probed by hand on the raw JSON.
    let plane_entry = planes
        .iter()
        .find(|p| p.data.pool == POOL)
        .expect("the registered pool has a PoolData write in its own ledger");
    assert_eq!(plane_entry.data.plane, PLANE);
    assert_eq!(plane_entry.data.reserves, vec!["100000000000", "30617317"]);

    // The instance carries the share token as STATE, at birth — the
    // fundamental source that needs no deposit.
    let inst = instances
        .iter()
        .find(|i| i.state.pool == POOL)
        .expect("the pool instance is written in the registration tx");
    assert_eq!(inst.state.token_share.as_deref(), Some(SHARE));
    assert_eq!(inst.state.plane.as_deref(), Some(PLANE));
}
