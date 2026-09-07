//! Raw mainnet registration ledgers → extraction → FULL STAGING for the
//! Soroswap adapter (task 0518). Validates on REAL history the assumption
//! the corroboration gate rests on: the factory deploys + initialises the
//! pair in the registering transaction, so every genuine `new_pair` has a
//! same-ledger CREATED pair instance pointing back at the emitter.
//!
//! Skipped when `FACTORY_PAIR_REG_LEDGERS` is unset — a directory of raw
//! `LedgerCloseMetaBatch` files named `<seq>.xdr`, fetched straight from the
//! public archive over HTTPS (no AWS tooling):
//!
//! ```sh
//! python3 - <<'EOF'
//! import urllib.request
//! def key(seq):
//!     start = seq - (seq % 64000); end = start + 63999
//!     ph = format(0xFFFFFFFF - start, '08X'); fh = format(0xFFFFFFFF - seq, '08X')
//!     return f"v1.1/stellar/ledgers/pubnet/{ph}--{start}-{end}/{fh}--{seq}.xdr.zst"
//! for seq in (50688706, 50746348, 63800724):
//!     urllib.request.urlretrieve(
//!         f"https://aws-public-blockchain.s3.amazonaws.com/{key(seq)}",
//!         f"{seq}.xdr.zst")
//! EOF
//! zstd -d *.xdr.zst
//! ```
//!
//! The three shipped picks cover the FIRST registration in history (dead
//! early factory, 50,688,706), the documented factory's first (50,746,348)
//! and the newest at harvest time (63,800,724) — three eras, two factory
//! generations.

use db_clickhouse::persist::ids;
use db_clickhouse::persist::stage;
use stellar_xdr::{LedgerCloseMetaBatch, Limits, ReadXdr};
use xdr_parser::pool_pair_factory::{detect_pair_registrations, extract_factory_pairs};
use xdr_parser::types::{ExtractedLedger, ExtractedTransaction};

#[test]
fn raw_registration_ledgers_stage_corroborated_registry_rows() {
    let Ok(dir) = std::env::var("FACTORY_PAIR_REG_LEDGERS") else {
        eprintln!("FACTORY_PAIR_REG_LEDGERS unset — skipping (see module docs)");
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
        "no .xdr files in FACTORY_PAIR_REG_LEDGERS"
    );

    for path in &files {
        let bytes = std::fs::read(path).expect("ledger readable");
        let batch =
            LedgerCloseMetaBatch::from_xdr(&bytes, Limits::none()).expect("a LedgerCloseMetaBatch");

        let mut pairs = Vec::new();
        let mut events: Vec<(String, Vec<xdr_parser::types::ExtractedEvent>)> = Vec::new();
        let mut txs: Vec<ExtractedTransaction> = Vec::new();
        let mut seq_out = 0u32;
        for lcm in batch.ledger_close_metas.iter() {
            seq_out = xdr_parser::meta::for_each_tx_meta(lcm, |seq, i, meta| {
                let hash = format!("{i:064x}");
                let changes = xdr_parser::extract_ledger_entry_changes(meta, &hash, seq, 0);
                pairs.extend(extract_factory_pairs(&changes));
                events.push((
                    hash.clone(),
                    xdr_parser::extract_events(meta, &hash, seq, 0),
                ));
                txs.push(synthetic_tx(&hash, seq));
            });
        }

        let regs = detect_pair_registrations(&events);
        assert!(
            !regs.is_empty(),
            "ledger {seq_out}: a registration ledger must yield new_pair events"
        );

        // The design assumption, checked on real history: every registered
        // pair's instance exists IN THIS LEDGER, CREATED, and points back at
        // the emitting factory.
        for reg in &regs {
            let inst = pairs
                .iter()
                .find(|p| p.state.pair == reg.event.pair)
                .unwrap_or_else(|| {
                    panic!(
                        "ledger {seq_out}: registered pair {} has no same-ledger instance",
                        reg.event.pair
                    )
                });
            assert!(
                inst.created,
                "ledger {seq_out}: pair {} instance is not a CREATION — the gate assumption breaks",
                reg.event.pair
            );
            assert_eq!(
                inst.state.factory, reg.factory,
                "ledger {seq_out}: pair declares a different factory than the emitter"
            );
            assert_eq!(
                (&inst.state.token_0, &inst.state.token_1),
                (&reg.event.token_0, &reg.event.token_1),
                "ledger {seq_out}: event legs and instance legs disagree"
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

        let writes: Vec<xdr_parser::pool_family::PoolFamilyWrite> = pairs
            .iter()
            .cloned()
            .map(xdr_parser::pool_family::PoolFamilyWrite::FactoryPair)
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

        // Every real registration must become a registry row with the
        // pair's own facts — none may be lost to the corroboration gate.
        for reg in &regs {
            let want = payload(&reg.event.pair);
            let row = staged
                .pool_rows
                .iter()
                .find(|r| r.pool_id == want && r.pool_kind == 1)
                .unwrap_or_else(|| {
                    panic!(
                        "ledger {seq_out}: registration {} did not stage a registry row",
                        reg.event.pair
                    )
                });
            assert_eq!(row.deployment_id, ids::contract_id(&reg.factory));
            assert_eq!(
                row.legs,
                vec![
                    ids::contract_id(&reg.event.token_0),
                    ids::contract_id(&reg.event.token_1)
                ]
            );
            assert_eq!(row.fee_bps, 30);
            // And the self-stamped declaration rides along.
            let decl = staged
                .pool_instance_state_rows
                .iter()
                .find(|r| r.pool_id == want)
                .expect("the pair's declaration staged");
            assert_eq!(decl.plane_id, ids::contract_id(&reg.event.pair));
            assert_eq!(decl.share_token_id, ids::contract_id(&reg.event.pair));
        }

        eprintln!(
            "ledger {seq_out}: {} registration(s) staged, {} pair instance write(s)",
            regs.len(),
            pairs.len()
        );
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

fn payload(strkey: &str) -> [u8; 32] {
    match stellar_strkey::Strkey::from_string(strkey) {
        Ok(stellar_strkey::Strkey::Contract(c)) => c.0,
        _ => panic!("not a contract strkey"),
    }
}
