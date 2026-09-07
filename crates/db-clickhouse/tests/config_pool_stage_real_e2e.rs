//! Raw mainnet registration ledgers → extraction → FULL STAGING for the
//! config-factory adapter (task 0518, third adapter). Validates on REAL
//! history the assumption the corroboration gate rests on: the factory
//! deploys + constructs the pool in the registering transaction, so every
//! genuine `create`/`liquidity_pool` has a same-ledger CREATED pool whose
//! own `CONFIG` names its legs, fee and share token.
//!
//! Skipped when `CONFIG_POOL_REG_LEDGERS` is unset — a directory of raw
//! `LedgerCloseMetaBatch` files named `<seq>.xdr`, fetched straight from the
//! public archive over HTTPS (no AWS tooling; the recipe in
//! `pair_factory_stage_real_e2e` works verbatim). The family has 20
//! registrations in ALL of history across SIX factory deployments — five
//! dead early ones with 6 pools (caught 2026-09-05 by cross-checking the
//! sibling project's registry; the first shape-wide harvest was scoped to
//! the documented factory) plus the documented factory with 14 — so the
//! shipped run covers every single one, the whole population, not picks:
//! 50,875,676 / 50,885,571 / 51,567,723 / 51,569,423 / 51,569,903 /
//! 51,571,828 / 51,572,026 / 51,572,030 / 51,572,101 / 51,927,948 /
//! 53,853,219 / 53,853,220 / 53,955,603 / 54,517,368 / 54,953,243 /
//! 54,953,245 / 54,953,247 / 54,953,248 / 63,293,708 / 64,030,567.

use db_clickhouse::persist::ids;
use db_clickhouse::persist::stage;
use stellar_xdr::{LedgerCloseMetaBatch, Limits, ReadXdr};
use xdr_parser::pool_config_factory::{
    detect_config_pool_registrations, extract_address_list_writes, extract_config_pools,
};
use xdr_parser::types::{ExtractedLedger, ExtractedTransaction};

#[test]
fn raw_registration_ledgers_stage_corroborated_registry_rows() {
    let Ok(dir) = std::env::var("CONFIG_POOL_REG_LEDGERS") else {
        eprintln!("CONFIG_POOL_REG_LEDGERS unset — skipping (see module docs)");
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("ledger dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "xdr"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no .xdr files in CONFIG_POOL_REG_LEDGERS"
    );

    let mut total_regs = 0usize;
    for path in &files {
        let bytes = std::fs::read(path).expect("ledger readable");
        let batch =
            LedgerCloseMetaBatch::from_xdr(&bytes, Limits::none()).expect("a LedgerCloseMetaBatch");

        let mut pools = Vec::new();
        let mut lists = Vec::new();
        let mut events: Vec<(String, Vec<xdr_parser::types::ExtractedEvent>)> = Vec::new();
        let mut txs: Vec<ExtractedTransaction> = Vec::new();
        let mut seq_out = 0u32;
        for lcm in batch.ledger_close_metas.iter() {
            seq_out = xdr_parser::meta::for_each_tx_meta(lcm, |seq, i, meta| {
                let hash = format!("{i:064x}");
                let changes = xdr_parser::extract_ledger_entry_changes(meta, &hash, seq, 0);
                pools.extend(extract_config_pools(&changes));
                lists.extend(extract_address_list_writes(&changes));
                events.push((
                    hash.clone(),
                    xdr_parser::extract_events(meta, &hash, seq, 0),
                ));
                txs.push(synthetic_tx(&hash, seq));
            });
        }

        let regs = detect_config_pool_registrations(&events);
        assert!(
            !regs.is_empty(),
            "ledger {seq_out}: a registration ledger must yield create/liquidity_pool events"
        );
        total_regs += regs.len();

        // The design assumption, checked on real history: every registered
        // pool wrote its own full CONFIG in this ledger, and its instance
        // was CREATED here.
        for reg in &regs {
            let cp = pools
                .iter()
                .find(|p| p.state.pool == reg.pool)
                .unwrap_or_else(|| {
                    panic!(
                        "ledger {seq_out}: registered pool {} has no same-ledger keyed write",
                        reg.pool
                    )
                });
            assert!(
                cp.created,
                "ledger {seq_out}: pool {} instance is not a CREATION — the gate assumption breaks",
                reg.pool
            );
            let config = cp.state.config.as_ref().unwrap_or_else(|| {
                panic!(
                    "ledger {seq_out}: pool {} wrote no decodable CONFIG at creation",
                    reg.pool
                )
            });
            assert!(
                cp.state.reserves.is_some(),
                "ledger {seq_out}: creation writes the zero reserve pair"
            );
            assert_ne!(config.token_a, config.token_b, "distinct legs");
            // The membership-list anchor, on real history: the EMITTER
            // recorded the pool in its own storage in the registering
            // ledger — the fact the two-stage gate's pointwise half rests
            // on, checked here across every factory-wasm generation.
            assert!(
                lists
                    .iter()
                    .any(|l| l.owner == reg.factory && l.members.contains(&reg.pool)),
                "ledger {seq_out}: the emitter did not record pool {} in its own \
                 address list — the membership-list gate assumption breaks",
                reg.pool
            );
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

        let writes: Vec<xdr_parser::pool_family::PoolFamilyWrite> = pools
            .iter()
            .cloned()
            .map(xdr_parser::pool_family::PoolFamilyWrite::ConfigPool)
            .chain(
                lists
                    .iter()
                    .cloned()
                    .map(xdr_parser::pool_family::PoolFamilyWrite::AddressList),
            )
            .collect();
        let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
            ledger: &ledger,
            transactions: &txs,
            operations: &ops,
            events: &events,
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

        // Every detected registration must survive corroboration into a
        // registry row carrying the pool's OWN facts.
        let soroban_rows: Vec<_> = staged
            .pool_rows
            .iter()
            .filter(|r| r.pool_kind == 1)
            .collect();
        assert_eq!(
            soroban_rows.len(),
            regs.len(),
            "ledger {seq_out}: every registration stages exactly one registry row"
        );
        for reg in &regs {
            let want = payload(&reg.pool);
            let config = pools
                .iter()
                .find(|p| p.state.pool == reg.pool)
                .and_then(|p| p.state.config.as_ref())
                .expect("checked above");
            let row = soroban_rows
                .iter()
                .find(|r| r.pool_id == want)
                .expect("the registration's row");
            assert_eq!(row.deployment_id, ids::contract_id(&reg.factory));
            assert_eq!(
                row.legs,
                vec![
                    ids::contract_id(&config.token_a),
                    ids::contract_id(&config.token_b)
                ]
            );
            assert_eq!(i64::from(row.fee_bps), config.total_fee_bps);
            assert_eq!(row.pool_type_raw, config.pool_type.to_string());
            // Creation state: the TRUE-zero reserve pair, self-stamped.
            let state = staged
                .pool_state_change_rows
                .iter()
                .find(|r| r.pool_id == want)
                .expect("the creation reserve row staged");
            assert_eq!(state.plane_id, ids::contract_id(&reg.pool));
            // The declaration with the SEPARATE share token.
            let decl = staged
                .pool_instance_state_rows
                .iter()
                .find(|r| r.pool_id == want)
                .expect("the pool's declaration staged");
            assert_eq!(decl.plane_id, ids::contract_id(&reg.pool));
            assert_eq!(decl.share_token_id, ids::contract_id(&config.share_token));
            assert_ne!(
                decl.share_token_id,
                ids::contract_id(&reg.pool),
                "the share token is a separate contract in this family"
            );
        }

        eprintln!(
            "ledger {seq_out}: {} registration(s) staged, {} keyed write(s)",
            regs.len(),
            pools.len()
        );
    }
    eprintln!("registrations staged across the corpus: {total_regs}");
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

fn payload(strkey: &str) -> [u8; 32] {
    match stellar_strkey::Strkey::from_string(strkey) {
        Ok(stellar_strkey::Strkey::Contract(c)) => c.0,
        _ => panic!("not a contract strkey"),
    }
}
