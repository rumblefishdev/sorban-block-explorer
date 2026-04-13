//! Per-ledger processing: parse all stages and persist atomically.

use sqlx::PgPool;
use std::time::Instant;
use stellar_xdr::curr::{LedgerCloseMeta, TransactionMeta};
use tracing::{info, warn};

use super::HandlerError;
use super::persist;

/// Process a single ledger: run all parsing stages and persist in one DB transaction.
pub async fn process_ledger(meta: &LedgerCloseMeta, pool: &PgPool) -> Result<(), HandlerError> {
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
    let mut all_tokens = Vec::new();

    for (_tx_hash, tx_source, changes) in &all_ledger_entry_changes {
        let deployments = xdr_parser::extract_contract_deployments(changes, tx_source);
        let tokens = xdr_parser::detect_tokens(&deployments);
        all_tokens.extend(tokens);
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
    let persist_timer = Instant::now();
    let mut db_tx = pool.begin().await?;

    persist::persist_ledger(
        &mut db_tx,
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
        &all_tokens,
        &all_nfts,
    )
    .await?;

    db_tx.commit().await?;

    let persist_ms = persist_timer.elapsed().as_millis();

    info!(
        ledger_sequence,
        tx_count = extracted_transactions.len(),
        parse_errors = tx_parse_errors.len(),
        parse_ms,
        persist_ms,
        "ledger saved to database"
    );

    Ok(())
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
