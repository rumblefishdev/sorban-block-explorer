//! Per-ledger processing: parse all stages.
//!
//! Pure parse half — [`parse_ledger`] runs every extract stage and
//! returns the slices in [`ParseOutput`]. No I/O, no DB.
//!
//! Persist half is driven directly from the Lambda handler in
//! [`super`] via `db_clickhouse::persist::persist_ledger_clickhouse`
//! — the same one-shot wrapper the runner's `Sink::persist_ledger`
//! fallback uses (open `PartitionWriter` → write_ledger → commit per
//! ledger).

use std::collections::HashMap;
use std::time::Instant;

use stellar_xdr::{LedgerCloseMeta, TransactionMeta};
use tracing::{info, warn};
use xdr_parser::types::{
    ExtractedAccountState, ExtractedAsset, ExtractedContractDeployment, ExtractedContractInterface,
    ExtractedEvent, ExtractedInvocation, ExtractedLedger, ExtractedLiquidityPool,
    ExtractedLiquidityPoolSnapshot, ExtractedLpPosition, ExtractedNft, ExtractedNftEvent,
    ExtractedOperation, ExtractedTransaction,
};

/// Parsed-but-not-yet-persisted output of a single `LedgerCloseMeta`.
///
/// Mirrors the slices the writers consume, packaged so a caller can
/// drive parse and persist as separate steps (task 0205 — CH backfill
/// sink calls `parse_ledger` then a CH-flavored persist).
#[derive(Debug, Clone)]
pub struct ParseOutput {
    pub ledger: ExtractedLedger,
    pub transactions: Vec<ExtractedTransaction>,
    pub operations: Vec<(String, Vec<ExtractedOperation>)>,
    pub events: Vec<(String, Vec<ExtractedEvent>)>,
    pub invocations: Vec<(String, Vec<ExtractedInvocation>)>,
    pub contract_interfaces: Vec<ExtractedContractInterface>,
    pub contract_deployments: Vec<ExtractedContractDeployment>,
    pub account_states: Vec<ExtractedAccountState>,
    pub liquidity_pools: Vec<ExtractedLiquidityPool>,
    pub pool_snapshots: Vec<ExtractedLiquidityPoolSnapshot>,
    pub assets: Vec<ExtractedAsset>,
    pub nfts: Vec<ExtractedNft>,
    pub nft_events: Vec<ExtractedNftEvent>,
    pub lp_positions: Vec<ExtractedLpPosition>,
    /// On-chain Soroban token metadata (name/symbol/decimals) from
    /// instance-storage `METADATA`, for the `soroban_contract_metadata` side
    /// table (task 0297). SACs already excluded by the producer.
    pub contract_metadata_writes: Vec<xdr_parser::ExtractedContractMetadata>,
    /// Per-holder Soroban token balances from `ContractData` `Balance(Address)`
    /// ledger entries, persisted into the unified `balances` table (task 0331; the
    /// field name is leftover Option-A naming — no `soroban_token_balances` table exists).
    pub soroban_token_balances: Vec<xdr_parser::ExtractedSorobanBalance>,
    /// Every pool family's state writes behind one seam (task 0518,
    /// decision 4a) — staging partitions by variant.
    pub pool_family_writes: Vec<xdr_parser::pool_family::PoolFamilyWrite>,
    /// Per-transaction operation tree JSON, collected by `extract_invocations`.
    /// Neither write path reads it today (CH writer skips it, PG flow
    /// took it as `_operation_trees`). Kept on `ParseOutput` so the
    /// downstream wiring stays live for the eventual consumer.
    #[allow(dead_code)]
    pub operation_trees: Vec<(String, serde_json::Value)>,
    /// Parse-half wall time in ms. Surfaced for diagnostic logs;
    /// backfill-runner tracks its own timings, the
    /// CH-write Lambda no longer logs this in-band.
    #[allow(dead_code)]
    pub parse_ms: u128,
    /// Per-tx indices skipped due to `parse_error`. Surfaced for
    /// diagnostic logs by the PG flow; CH-write Lambda flag-skips
    /// them transparently via per-tx `parse_error` field on
    /// `ExtractedTransaction`.
    #[allow(dead_code)]
    pub tx_parse_errors: Vec<usize>,
    /// Task 0218 / 0220 — SAC overrides derived from the classic-asset
    /// slice (`assets`).
    pub sac_overrides: Vec<xdr_parser::SacOverride>,
}

/// Error returned by [`init_network_id`] when the
/// `STELLAR_NETWORK_PASSPHRASE` env var is missing or empty. Mirrors
/// the `MtlsError::MissingEnv` shape so the Lambda cold-start path
/// can surface both failures uniformly via `error!` + `Init Errors`
/// CW metric.
#[derive(Debug, thiserror::Error)]
#[error(
    "STELLAR_NETWORK_PASSPHRASE env not set or empty; SAC contract_id derivation cannot proceed"
)]
pub struct NetworkIdError;

/// Eager-initialise the network-id hash at cold start. Idempotent —
/// subsequent calls return the cached value. Lambda binary calls this
/// in `main()` BEFORE `lambda_runtime::run` so a missing passphrase
/// fails the Lambda init (CW `Init Errors`), not the first per-event
/// `parse_ledger`. Backfill-runner calls it from
/// their own startup before driving the parse loop.
///
/// **Whitespace handling**: `SHA256(b"   ")` is a deterministic but
/// meaningless network id that would silently derive wrong SAC
/// contract_ids; we reject whitespace-only input the same way we
/// reject an unset variable. If trimming changed the length we
/// warn-log because that suggests an env-injection bug (trailing
/// newline / leading space) the operator should fix even though we
/// recover transparently.
///
/// The value itself is cached ONCE in [`xdr_parser::net_id`] — the single
/// process-global source of truth (audit A4 removed a duplicate `OnceLock` that
/// lived here). Its `OnceLock` pins the passphrase on the first call; tests that
/// need a specific passphrase set `STELLAR_NETWORK_PASSPHRASE` before any
/// `parse_ledger` (or call `xdr_parser::network_id(...)` directly with their
/// fixture value).
pub fn init_network_id() -> Result<&'static [u8; 32], NetworkIdError> {
    // Validation + drift warning stay here (indexer cold-start policy); only the
    // cache moved to `xdr_parser::net_id()`.
    let raw = std::env::var("STELLAR_NETWORK_PASSPHRASE").map_err(|_| NetworkIdError)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(NetworkIdError);
    }
    if trimmed.len() != raw.len() {
        warn!(
            raw_len = raw.len(),
            trimmed_len = trimmed.len(),
            "STELLAR_NETWORK_PASSPHRASE had leading/trailing whitespace; \
             trimmed before hashing — fix the env injection to avoid drift"
        );
    }
    // Triggers + caches the shared accessor (which trims identically). `None` only
    // if the env vanished between our read and the accessor's — not possible in
    // practice, mapped to the same error for safety.
    xdr_parser::net_id().ok_or(NetworkIdError)
}

/// Network identifier hash. Delegates to the shared [`xdr_parser::net_id`] cache.
/// Falls back to lazy init from `STELLAR_NETWORK_PASSPHRASE` for callers that did
/// not run [`init_network_id`] in their cold-start path (legacy tests, dev tools).
/// Production Lambda always pre-inits, so this lazy branch is dead code in the hot
/// path.
fn network_id() -> &'static [u8; 32] {
    xdr_parser::net_id().unwrap_or_else(|| {
        panic!(
            "STELLAR_NETWORK_PASSPHRASE env not set; call \
             `init_network_id()` from the binary's cold-start path \
             before any `parse_ledger` invocation."
        )
    })
}

/// Pure parse half — runs every extract stage and returns the slices.
/// No I/O, no DB.
pub fn parse_ledger(meta: &LedgerCloseMeta) -> ParseOutput {
    let extracted_ledger = xdr_parser::extract_ledger(meta);
    let ledger_sequence = extracted_ledger.sequence;
    let closed_at = extracted_ledger.closed_at;

    let parse_timer = Instant::now();

    info!(ledger_sequence, "parsing ledger");

    let net_id = network_id();
    let extracted_transactions =
        xdr_parser::extract_transactions(meta, ledger_sequence, closed_at, net_id);

    let envelopes = xdr_parser::envelope::extract_envelopes(meta, net_id);
    let tx_metas = collect_tx_metas(meta);
    let tx_results = xdr_parser::collect_tx_results(meta);

    let mut all_operations = Vec::new();
    let mut all_events = Vec::new();
    let mut all_invocations = Vec::new();
    let mut all_operation_trees: Vec<(String, serde_json::Value)> = Vec::new();
    let mut all_contract_interfaces = Vec::new();
    let mut all_ledger_entry_changes = Vec::new();
    let mut all_nft_events = Vec::new();
    let mut tx_parse_errors = Vec::new();

    for (tx_index, ext_tx) in extracted_transactions.iter().enumerate() {
        if ext_tx.parse_error {
            warn!(
                ledger_sequence,
                tx_index,
                tx_hash = %ext_tx.hash,
                "skipping per-tx parsing for parse_error transaction"
            );
            tx_parse_errors.push(tx_index);
            continue;
        }

        let envelope = envelopes.get(tx_index).and_then(Option::as_ref);
        let tx_meta = tx_metas.get(tx_index).copied();

        if let Some(env) = envelope {
            let inner = xdr_parser::envelope::inner_transaction(env);
            let op_results = tx_results
                .get(tx_index)
                .and_then(|r| xdr_parser::tx_op_results(r));
            let ops = xdr_parser::extract_operations(
                &inner,
                tx_meta,
                op_results,
                &ext_tx.hash,
                ledger_sequence,
                tx_index,
            );
            all_operations.push((ext_tx.hash.clone(), ops));
        }

        if let Some(tm) = tx_meta {
            let events = xdr_parser::extract_events(tm, &ext_tx.hash, ledger_sequence, closed_at);
            let nft_events = xdr_parser::detect_nft_events(&events, net_id);
            all_nft_events.extend(nft_events);
            all_events.push((ext_tx.hash.clone(), events));

            if let Some(env) = envelope {
                let inner = xdr_parser::envelope::inner_transaction(env);
                let inv_result = xdr_parser::extract_invocations(
                    &inner,
                    Some(tm),
                    &ext_tx.hash,
                    ledger_sequence,
                    closed_at,
                    &ext_tx.source_account,
                    ext_tx.successful,
                );
                all_invocations.push((ext_tx.hash.clone(), inv_result.invocations));
                if let Some(tree) = inv_result.operation_tree {
                    all_operation_trees.push((ext_tx.hash.clone(), tree));
                }
            }

            let interfaces = xdr_parser::extract_contract_interfaces(tm);
            all_contract_interfaces.extend(interfaces);

            let changes = xdr_parser::extract_ledger_entry_changes(
                tm,
                &ext_tx.hash,
                ledger_sequence,
                closed_at,
            );
            all_ledger_entry_changes.push((
                ext_tx.hash.clone(),
                ext_tx.source_account.clone(),
                changes,
            ));
        }
    }

    let mut all_contract_deployments = Vec::new();
    let mut all_account_states = Vec::new();
    let mut all_liquidity_pools = Vec::new();
    let mut all_pool_snapshots = Vec::new();
    let mut all_assets = Vec::new();
    let mut all_lp_positions = Vec::new();

    let sac_identity_by_contract: HashMap<String, xdr_parser::SacAssetIdentity> =
        extracted_transactions
            .iter()
            .enumerate()
            .filter(|(_, ext_tx)| !ext_tx.parse_error)
            .filter_map(|(tx_index, _)| envelopes.get(tx_index).and_then(Option::as_ref))
            .flat_map(|env| {
                let inner = xdr_parser::envelope::inner_transaction(env);
                xdr_parser::extract_sac_identities(&inner, net_id)
            })
            .collect();

    // Task 0255 Phase 1 — per-contract op-source override for the deployer
    // attribution path. Mirrors the SAC walker shape: walks every successful
    // envelope, collects `(contract_id, deployer_strkey)` pairs from
    // top-level `CreateContract*` ops AND nested `CreateContractHostFn`
    // auth entries. The map drives `extract_contract_deployments`'s
    // `deployer_account` fallback — present entries override the tx
    // source, absent entries inherit it (the ~88 % of mainnet deploys
    // with no per-op source override).
    let deployer_by_contract: HashMap<String, String> = extracted_transactions
        .iter()
        .enumerate()
        .filter(|(_, ext_tx)| !ext_tx.parse_error)
        .filter_map(|(tx_index, _)| envelopes.get(tx_index).and_then(Option::as_ref))
        .flat_map(|env| {
            let inner = xdr_parser::envelope::inner_transaction(env);
            let tx_source = inner.source_account();
            xdr_parser::extract_op_source_per_contract(&inner, &tx_source, net_id)
        })
        .collect();

    let mut all_contract_metadata_writes: Vec<xdr_parser::ExtractedContractMetadata> = Vec::new();
    let mut all_soroban_token_balances: Vec<xdr_parser::ExtractedSorobanBalance> = Vec::new();
    let mut all_pool_family_writes: Vec<xdr_parser::pool_family::PoolFamilyWrite> = Vec::new();
    for (_tx_hash, tx_source, changes) in &all_ledger_entry_changes {
        let deployments = xdr_parser::extract_contract_deployments(
            changes,
            tx_source,
            &sac_identity_by_contract,
            &deployer_by_contract,
        );
        // detect_assets uses WASM interfaces to classify non-SAC deployments
        // as Soroban-native assets (task 0120). Contracts deployed in this
        // ledger without a matching interface are skipped here and picked up
        // by the late-WASM bridge step (task 0120) once reclassify promotes
        // them.
        let assets = xdr_parser::detect_assets(&deployments, &all_contract_interfaces);
        all_assets.extend(assets);
        all_pool_family_writes.extend(xdr_parser::pool_family::extract_pool_family_writes(changes));
        let classic_credits = xdr_parser::detect_classic_credit_assets(changes);
        all_assets.extend(classic_credits);
        all_contract_deployments.extend(deployments);

        let accounts = xdr_parser::extract_account_states(changes);
        all_account_states.extend(accounts);

        let (pools, snapshots) = xdr_parser::extract_liquidity_pools(changes);
        all_liquidity_pools.extend(pools);
        all_pool_snapshots.extend(snapshots);

        let lp_pos = xdr_parser::extract_lp_positions(changes);
        all_lp_positions.extend(lp_pos);

        all_contract_metadata_writes.extend(xdr_parser::extract_contract_metadata_writes(changes));
        all_soroban_token_balances.extend(xdr_parser::extract_soroban_token_balances(changes));
    }

    all_assets.push(xdr_parser::native_asset_singleton());

    let all_nfts = xdr_parser::detect_nfts(&all_nft_events);

    // Un-deployed-SAC overrides, sourced from the ledger's EVENTS (task 0323).
    // An un-deployed SAC emits a CAP-67 unified event under its reserved C…
    // address; the crypto gate (`derive_sac(asset) == emitter`) proves it, and
    // staging then models it as an ASSET, not a `soroban_contracts` row (the
    // override id suppresses the Pass-2 FK stub; its identity seeds the assets
    // row). Replaces the trustline-sourced `derive_sac_overrides_from_assets`:
    // an un-deployed SAC matters once it has activity (an event), not at the
    // trustline — and trustline-only SACs keep their classic-credit asset row.
    let sac_overrides = xdr_parser::detect_undeployed_sac_overrides(&all_events, net_id);

    let parse_ms = parse_timer.elapsed().as_millis();

    let nft_events = xdr_parser::extract_nft_ownership_events(&all_nft_events);

    ParseOutput {
        ledger: extracted_ledger,
        transactions: extracted_transactions,
        operations: all_operations,
        events: all_events,
        invocations: all_invocations,
        contract_interfaces: all_contract_interfaces,
        contract_deployments: all_contract_deployments,
        account_states: all_account_states,
        liquidity_pools: all_liquidity_pools,
        // lore-0356: collapse to one snapshot per (pool, ledger) = end-of-ledger
        // reserves. `extract_liquidity_pools` runs per-tx (emitting one snapshot
        // per mutating change), so dedup must happen here at ledger scope after
        // aggregating every tx's snapshots — a pool touched by >1 tx cannot be
        // deduped inside a single per-tx call.
        pool_snapshots: xdr_parser::dedup_final_pool_snapshots(all_pool_snapshots),
        assets: all_assets,
        nfts: all_nfts,
        nft_events,
        lp_positions: all_lp_positions,
        contract_metadata_writes: all_contract_metadata_writes,
        soroban_token_balances: all_soroban_token_balances,
        // Plane writes and instance images pass through unfolded: staging
        // owns the one fold per destination table
        // (`fold_pool_state_changes` / `fold_pool_instance_state`).
        pool_family_writes: all_pool_family_writes,
        operation_trees: all_operation_trees,
        parse_ms,
        tx_parse_errors,
        sac_overrides,
    }
}

/// Collect per-transaction TransactionMeta references from any LedgerCloseMeta variant.
fn collect_tx_metas(meta: &LedgerCloseMeta) -> Vec<&TransactionMeta> {
    match meta {
        LedgerCloseMeta::V0(v) => v
            .tx_processing
            .iter()
            .map(|p| &p.tx_apply_processing)
            .collect(),
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
