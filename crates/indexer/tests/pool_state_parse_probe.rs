//! Env-gated probe (task 0374 e2e): does `parse_ledger` itself surface the
//! pool-state extractions on a REAL raw ledger? The corpus tests call the
//! extractors directly, and the staging e2e starts from extracted values —
//! this is the one seam neither covers.
#[test]
fn parse_ledger_surfaces_pool_state() {
    let Ok(path) = std::env::var("POOL_STATE_LEDGER") else {
        eprintln!("POOL_STATE_LEDGER unset — skipping");
        return;
    };
    // SAFETY: same env the runner sets; required by parse_ledger's cold-start
    // contract.
    unsafe {
        std::env::set_var(
            "STELLAR_NETWORK_PASSPHRASE",
            "Public Global Stellar Network ; September 2015",
        );
    }
    indexer::handler::process::init_network_id().expect("network id");
    let raw = std::fs::read(&path).expect("read raw ledger");
    let batch = xdr_parser::deserialize_batch(&raw).expect("batch");
    for meta in batch.ledger_close_metas.iter() {
        let parsed = indexer::handler::process::parse_ledger(meta);
        use xdr_parser::pool_family::PoolFamilyWrite;
        let planes = parsed
            .pool_family_writes
            .iter()
            .filter(|w| matches!(w, PoolFamilyWrite::RouterPlane(_)))
            .count();
        let instances = parsed
            .pool_family_writes
            .iter()
            .filter(|w| matches!(w, PoolFamilyWrite::RouterPool(_)))
            .count();
        eprintln!(
            "ledger {}: plane_pool_data={} pool_instances={} balances={} events={}",
            parsed.ledger.sequence,
            planes,
            instances,
            parsed.soroban_token_balances.len(),
            parsed.events.len(),
        );
        assert!(
            planes + instances > 0,
            "hot-era ledger must surface pool state through parse_ledger"
        );
        // The exact seam the backfill sink drives: parsed → staging.
        let staged = db_clickhouse::persist::stage::prepare_with_sac_overrides(
            &db_clickhouse::persist::stage::StageInputs {
                ledger: &parsed.ledger,
                transactions: &parsed.transactions,
                operations: &parsed.operations,
                events: &parsed.events,
                invocations: &parsed.invocations,
                contract_interfaces: &parsed.contract_interfaces,
                contract_deployments: &parsed.contract_deployments,
                account_states: &parsed.account_states,
                liquidity_pools: &parsed.liquidity_pools,
                pool_snapshots: &parsed.pool_snapshots,
                assets: &parsed.assets,
                nfts: &parsed.nfts,
                nft_events: &parsed.nft_events,
                lp_positions: &parsed.lp_positions,
                contract_metadata_writes: &parsed.contract_metadata_writes,
                soroban_token_balances: &parsed.soroban_token_balances,
                pool_family_writes: &parsed.pool_family_writes,
                sac_classic: &std::collections::HashMap::new(),
                sac_overrides: &parsed.sac_overrides,
                prior_wasm_verdicts: &std::collections::HashMap::new(),
                prior_contract_verdicts: &std::collections::HashMap::new(),
                prior_contract_rows: &std::collections::HashMap::new(),
            },
        )
        .expect("staging");
        eprintln!(
            "ledger {}: staged pool_state_change_rows={} pool_instance_state_rows={}",
            parsed.ledger.sequence,
            staged.pool_state_change_rows.len(),
            staged.pool_instance_state_rows.len(),
        );
        assert!(
            !staged.pool_state_change_rows.is_empty(),
            "staging must emit pool state rows for a hot-era ledger"
        );
    }
}

/// Env-gated: every locally derived pool→share-token surrogate equals the
/// surrogate of the LIVE chain's `TokenShare` address (task 0374 e2e test 6).
#[test]
fn share_token_surrogates_match_chain() {
    let Ok(path) = std::env::var("SHARE_MAP_TSV") else {
        eprintln!("SHARE_MAP_TSV unset — skipping");
        return;
    };
    let raw = std::fs::read_to_string(&path).expect("map readable");
    let (mut ok, mut bad) = (0u32, 0u32);
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let mut it = line.split('\t');
        let (pool, chain_tok, our_id) = (
            it.next().unwrap(),
            it.next().unwrap(),
            it.next().unwrap().parse::<i64>().unwrap(),
        );
        if db_clickhouse::persist::ids::contract_id(chain_tok) == our_id {
            ok += 1;
        } else {
            bad += 1;
            eprintln!("MISMATCH pool={pool} chain={chain_tok} ours={our_id}");
        }
    }
    eprintln!("share surrogates vs chain: ok={ok} bad={bad}");
    assert_eq!(bad, 0);
    assert!(ok > 0);
}
