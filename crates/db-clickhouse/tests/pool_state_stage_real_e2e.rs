//! Raw mainnet ledger → extraction → FULL STAGING → final rows
//! (task 0374, step 7). The extractor corpora stop at the extractors; this
//! crosses the staging boundary too — payload decode, tx-id mapping, the
//! i128 parse — and asserts the exact rows the writer would insert, against
//! ground truth probed by hand on the registration ledger 63,893,403.
//!
//! Skipped when `POOL_STATE_LEDGER` is unset (same fixture as
//! `pool_state_real_ledger` in xdr-parser).

use db_clickhouse::persist::ids;
use db_clickhouse::persist::stage;
use stellar_xdr::{LedgerCloseMetaBatch, Limits, ReadXdr};
use xdr_parser::pool_state::{extract_plane_pool_data, extract_pool_instances};
use xdr_parser::types::{ExtractedLedger, ExtractedTransaction};

const POOL: &str = "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH";
const PLANE: &str = "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY";
const SHARE: &str = "CC5PU23MKXHUFJKGG5FAUG7MFZX2KMWXPNZP26DDYW76VCB26UWMPEI6";

#[test]
fn raw_registration_ledger_stages_the_exact_rows() {
    let Ok(path) = std::env::var("POOL_STATE_LEDGER") else {
        eprintln!("POOL_STATE_LEDGER unset — skipping");
        return;
    };
    let bytes = std::fs::read(&path).expect("ledger readable");
    let batch =
        LedgerCloseMetaBatch::from_xdr(&bytes, Limits::none()).expect("a LedgerCloseMetaBatch");

    let mut planes = Vec::new();
    let mut instances = Vec::new();
    let mut txs: Vec<ExtractedTransaction> = Vec::new();
    let mut seq_out = 0u32;
    for lcm in batch.ledger_close_metas.iter() {
        seq_out = xdr_parser::meta::for_each_tx_meta(lcm, |seq, i, meta| {
            let hash = format!("{i:064x}");
            let changes = xdr_parser::extract_ledger_entry_changes(meta, &hash, seq, 0);
            planes.extend(extract_plane_pool_data(&changes));
            instances.extend(extract_pool_instances(&changes));
            txs.push(synthetic_tx(&hash, seq));
        });
    }

    let ledger = ExtractedLedger {
        sequence: seq_out,
        hash: "00".repeat(32),
        closed_at: 0,
        protocol_version: 21,
        transaction_count: txs.len() as u32,
        base_fee: 100,
    };
    let ops: Vec<(String, Vec<xdr_parser::ExtractedOperation>)> =
        txs.iter().map(|t| (t.hash.clone(), vec![])).collect();

    let writes: Vec<xdr_parser::pool_family::PoolFamilyWrite> = planes
        .iter()
        .cloned()
        .map(xdr_parser::pool_family::PoolFamilyWrite::RouterPlane)
        .chain(
            instances
                .into_iter()
                .map(xdr_parser::pool_family::PoolFamilyWrite::RouterPool),
        )
        .collect();
    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: &txs,
        operations: &ops,
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &writes,
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("staging the raw ledger succeeds");

    eprintln!(
        "\n--- staged from raw: {} snapshot rows, {} instance-state rows ---",
        staged.pool_state_change_rows.len(),
        staged.pool_instance_state_rows.len()
    );

    // Every extracted plane write must survive staging — a refused row here
    // is silent loss past the extractor.
    assert_eq!(
        staged.pool_state_change_rows.len(),
        planes.len(),
        "extractor rows and staged rows must be 1:1"
    );

    // The registered pool's snapshot, value for value.
    let want_pool = payload(POOL);
    let snap = staged
        .pool_state_change_rows
        .iter()
        .find(|r| r.pool_id == want_pool)
        .expect("the new pool's snapshot staged");
    assert_eq!(snap.reserves, vec![100000000000i128, 30617317i128]);
    assert_eq!(snap.plane_id, ids::contract_id(PLANE));
    assert_eq!(snap.ledger_sequence, i64::from(seq_out));

    // The share relation, from STATE, no deposit involved.
    let inst = staged
        .pool_instance_state_rows
        .iter()
        .find(|r| r.pool_id == want_pool)
        .expect("the instance state staged from the instance entry");
    assert_eq!(inst.share_token_id, ids::contract_id(SHARE));
    assert_eq!(
        inst.plane_id,
        ids::contract_id(PLANE),
        "the pool's declared plane — the authority reserve reads filter on"
    );
}

fn payload(strkey: &str) -> [u8; 32] {
    match stellar_strkey::Strkey::from_string(strkey) {
        Ok(stellar_strkey::Strkey::Contract(c)) => c.0,
        _ => panic!("not a contract strkey"),
    }
}

fn synthetic_tx(hash: &str, seq: u32) -> ExtractedTransaction {
    ExtractedTransaction {
        hash: hash.to_string(),
        inner_tx_hash: None,
        ledger_sequence: seq,
        source_account: "G".to_string() + &"A".repeat(55),
        fee_source: None,
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".into(),
        envelope_xdr: String::new(),
        result_xdr: String::new(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: 0,
        parse_error: false,
        ledger_deltas: Vec::new(),
    }
}
