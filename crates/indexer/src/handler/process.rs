//! Per-ledger processing: parse all stages and persist atomically.

use aws_sdk_cloudwatch::{
    Client as CloudWatchClient,
    types::{Dimension, MetricDatum, StandardUnit},
};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;
use stellar_xdr::curr::{LedgerCloseMeta, TransactionMeta};
use tracing::{info, warn};

use super::HandlerError;
use super::persist;
use super::persist::ClassificationCache;

/// Network identifier hash, derived lazily from the
/// `STELLAR_NETWORK_PASSPHRASE` env var. Required for SAC contract_id
/// derivation (`SHA256(network_id || XDR(ContractIdPreimage))`).
///
/// The same passphrase is already a CDK config field
/// (`stellarNetworkPassphrase` in `infra/src/lib/types.ts`) used by the
/// ingestion stack to map Galexie's S3 prefix; we read it directly so
/// there is one source of truth for the network the indexer is targeting.
///
/// **Fail-fast on missing env**: silently defaulting to mainnet on a
/// testnet stack would derive wrong contract_ids and corrupt the assets
/// table (no SAC row would ever match a deployed contract). Panic at
/// first use is the loud, recoverable failure mode.
fn network_id() -> &'static [u8; 32] {
    static NETWORK_ID: OnceLock<[u8; 32]> = OnceLock::new();
    NETWORK_ID.get_or_init(|| {
        let passphrase = std::env::var("STELLAR_NETWORK_PASSPHRASE").unwrap_or_else(|_| {
            panic!(
                "STELLAR_NETWORK_PASSPHRASE env not set; SAC contract_id derivation \
                 cannot proceed. Expected the full Stellar passphrase string \
                 (e.g. \"Public Global Stellar Network ; September 2015\")."
            )
        });
        xdr_parser::network_id(&passphrase)
    })
}

/// Process a single ledger: run all parsing stages and persist in one DB transaction.
/// If `cw_client` is provided, publishes `LastProcessedLedgerSequence` to CloudWatch.
/// `classification_cache` is the per-worker NFT-filter cache (task 0118 Phase 2);
/// callers reuse the same instance across ledgers so it accumulates.
pub async fn process_ledger(
    meta: &LedgerCloseMeta,
    pool: &PgPool,
    cw_client: Option<&CloudWatchClient>,
    classification_cache: &ClassificationCache,
) -> Result<(), HandlerError> {
    // --- Stage 0024: Ledger + transaction extraction ---
    let extracted_ledger = xdr_parser::extract_ledger(meta)?;
    let ledger_sequence = extracted_ledger.sequence;
    let closed_at = extracted_ledger.closed_at;

    let parse_timer = Instant::now();

    info!(ledger_sequence, "parsing ledger");

    let extracted_transactions = xdr_parser::extract_transactions(meta, ledger_sequence, closed_at);

    // Get envelopes and per-tx metas for downstream stages
    let envelopes = xdr_parser::envelope::extract_envelopes(meta);
    let tx_metas = collect_tx_metas(meta);

    // Per-transaction parsing (stages 0025, 0026, 0027)
    let mut all_operations = Vec::new();
    let mut all_events = Vec::new();
    let mut all_invocations = Vec::new();
    let mut all_operation_trees = Vec::new();
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

        let envelope = envelopes.get(tx_index);
        let tx_meta = tx_metas.get(tx_index).copied();

        // --- Stage 0025: Operation extraction ---
        if let Some(env) = envelope {
            let inner = xdr_parser::envelope::inner_transaction(env);
            let ops = xdr_parser::extract_operations(
                &inner,
                tx_meta,
                &ext_tx.hash,
                ledger_sequence,
                tx_index,
            );
            all_operations.push((ext_tx.hash.clone(), ops));
        }

        // --- Stage 0026: Events, invocations, contract interfaces ---
        if let Some(tm) = tx_meta {
            let events = xdr_parser::extract_events(tm, &ext_tx.hash, ledger_sequence, closed_at);
            let nft_events = xdr_parser::detect_nft_events(&events);
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

            // --- Stage 0027: Ledger entry changes + derived state ---
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

    // Derive state from ledger entry changes (0027)
    let mut all_contract_deployments = Vec::new();
    let mut all_account_states = Vec::new();
    let mut all_liquidity_pools = Vec::new();
    let mut all_pool_snapshots = Vec::new();
    let mut all_assets = Vec::new();

    // Task 0160 — correlate SAC deployments with their underlying classic
    // asset. Each SAC's `contract_id` is deterministically derived from the
    // `ContractIdPreimage` per stellar-core (`SHA256(network_id || XDR)`),
    // so we key on the derived contract_id rather than `tx_hash`. That kills
    // multi-SAC/tx ambiguity and batch-boundary fragility in one stroke:
    // preimages come from both top-level `CreateContract` operations AND
    // `CreateContractHostFn` auth entries (factory pattern).
    let net_id = network_id();
    let sac_identity_by_contract: HashMap<String, xdr_parser::SacAssetIdentity> =
        extracted_transactions
            .iter()
            .enumerate()
            .filter(|(_, ext_tx)| !ext_tx.parse_error)
            .filter_map(|(tx_index, _)| envelopes.get(tx_index))
            .flat_map(|env| {
                let inner = xdr_parser::envelope::inner_transaction(env);
                xdr_parser::extract_sac_identities(&inner, net_id)
            })
            .collect();

    for (_tx_hash, tx_source, changes) in &all_ledger_entry_changes {
        let deployments =
            xdr_parser::extract_contract_deployments(changes, tx_source, &sac_identity_by_contract);
        // detect_assets uses WASM interfaces to classify non-SAC deployments
        // as Soroban-native assets (task 0120). Contracts deployed in this
        // ledger without a matching interface are skipped here and picked up
        // by the late-WASM bridge step (task 0120) once reclassify promotes
        // them.
        let assets = xdr_parser::detect_assets(&deployments, &all_contract_interfaces);
        all_assets.extend(assets);
        all_contract_deployments.extend(deployments);

        let accounts = xdr_parser::extract_account_states(changes);
        all_account_states.extend(accounts);

        let (pools, snapshots) = xdr_parser::extract_liquidity_pools(changes);
        all_liquidity_pools.extend(pools);
        all_pool_snapshots.extend(snapshots);
    }

    let all_nfts = xdr_parser::detect_nfts(&all_nft_events);

    let parse_ms = parse_timer.elapsed().as_millis();

    // --- Step 4: Atomic database transaction ---
    //
    // persist_ledger owns the transaction lifecycle (open/commit/retry) so that
    // transient 40001/40P01 conflicts replay the full envelope.
    //
    // Signature extension params (task 0149) — the parser does not yet produce
    // these; pass empty slices so wiring is in place end-to-end:
    //   * nft_events        → `nft_ownership` rows (follow-up from 0118)
    //   * lp_positions      → `lp_positions` rows  (task 0126)
    //   * inner_tx_hashes   → `transactions.inner_tx_hash` (follow-up parser work)
    let nft_events: Vec<xdr_parser::types::ExtractedNftEvent> = Vec::new();
    let lp_positions: Vec<xdr_parser::types::ExtractedLpPosition> = Vec::new();
    let inner_tx_hashes: HashMap<String, Option<String>> = HashMap::new();

    let persist_timer = Instant::now();
    persist::persist_ledger(
        pool,
        &extracted_ledger,
        &extracted_transactions,
        &all_operations,
        &all_events,
        &all_invocations,
        &all_operation_trees,
        &all_contract_interfaces,
        &all_contract_deployments,
        &all_account_states,
        &all_liquidity_pools,
        &all_pool_snapshots,
        &all_assets,
        &all_nfts,
        &nft_events,
        &lp_positions,
        &inner_tx_hashes,
        classification_cache,
    )
    .await?;
    let persist_ms = persist_timer.elapsed().as_millis();

    info!(
        ledger_sequence,
        tx_count = extracted_transactions.len(),
        parse_errors = tx_parse_errors.len(),
        parse_ms,
        persist_ms,
        "ledger saved to database"
    );

    if let Some(cw) = cw_client {
        publish_ledger_sequence_metric(cw, ledger_sequence).await;
    }

    Ok(())
}

/// Publish `LastProcessedLedgerSequence` to CloudWatch.
/// Best-effort: failures are logged as warnings and do not abort ledger processing.
async fn publish_ledger_sequence_metric(cw_client: &CloudWatchClient, ledger_sequence: u32) {
    let env_name = std::env::var("ENV_NAME").unwrap_or_else(|_| "unknown".to_string());
    let datum = MetricDatum::builder()
        .metric_name("LastProcessedLedgerSequence")
        .dimensions(
            Dimension::builder()
                .name("Environment")
                .value(&env_name)
                .build(),
        )
        .value(f64::from(ledger_sequence))
        .unit(StandardUnit::None)
        .build();
    let result = cw_client
        .put_metric_data()
        .namespace("SorobanBlockExplorer/Indexer")
        .metric_data(datum)
        .send()
        .await;
    if let Err(e) = result {
        warn!(ledger_sequence, error = %e, "failed to publish LastProcessedLedgerSequence metric");
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
