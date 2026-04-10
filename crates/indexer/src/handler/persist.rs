//! Persistence layer: writes all parsed data within a single DB transaction.

use std::collections::HashMap;
use tracing::warn;

use super::HandlerError;
use super::convert;
use xdr_parser::types::{
    ExtractedAccountState, ExtractedContractDeployment, ExtractedContractInterface, ExtractedEvent,
    ExtractedInvocation, ExtractedLedger, ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot,
    ExtractedNft, ExtractedOperation, ExtractedToken, ExtractedTransaction,
};

/// Persist all parsed data for a single ledger within `db_tx`.
///
/// The caller is responsible for calling `db_tx.commit()` on success.
#[allow(clippy::too_many_arguments)]
pub async fn persist_ledger(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ledger: &ExtractedLedger,
    transactions: &[ExtractedTransaction],
    operations: &[(String, Vec<ExtractedOperation>)],
    events: &[(String, Vec<ExtractedEvent>)],
    invocations: &[(String, Vec<ExtractedInvocation>)],
    operation_trees: &[(String, serde_json::Value)],
    contract_interfaces: &[ExtractedContractInterface],
    contract_deployments: &[ExtractedContractDeployment],
    account_states: &[ExtractedAccountState],
    liquidity_pools: &[ExtractedLiquidityPool],
    pool_snapshots: &[ExtractedLiquidityPoolSnapshot],
    tokens: &[ExtractedToken],
    nfts: &[ExtractedNft],
) -> Result<(), HandlerError> {
    // 1. Insert ledger
    let domain_ledger = convert::to_ledger(ledger);
    db::persistence::insert_ledger(&mut **db_tx, &domain_ledger).await?;

    // 2. Insert transactions and collect hash→id mapping
    let domain_txs: Vec<_> = transactions.iter().map(convert::to_transaction).collect();
    let tx_ids = db::persistence::insert_transactions_batch(&mut **db_tx, &domain_txs).await?;

    let hash_to_id: HashMap<&str, i64> = tx_ids.iter().map(|(h, id)| (h.as_str(), *id)).collect();
    let hash_to_source: HashMap<&str, &str> = transactions
        .iter()
        .map(|t| (t.hash.as_str(), t.source_account.as_str()))
        .collect();

    // 3. Insert operations — flatten all transactions into a single batch
    {
        let mut all_ops = Vec::new();
        for (tx_hash, ops) in operations {
            let Some(&tx_id) = hash_to_id.get(tx_hash.as_str()) else {
                warn!(tx_hash, "no transaction_id found for operations — skipping");
                continue;
            };
            let tx_source = hash_to_source.get(tx_hash.as_str()).copied().unwrap_or("");
            all_ops.extend(
                ops.iter()
                    .map(|op| convert::to_operation(op, tx_id, tx_source)),
            );
        }
        db::persistence::insert_operations_batch(&mut **db_tx, &all_ops).await?;
    }

    // 3b. Ensure all referenced contracts exist (FK constraint on soroban_events/invocations)
    {
        let mut contract_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (_tx_hash, evts) in events {
            for e in evts {
                if let Some(ref cid) = e.contract_id {
                    contract_ids.insert(cid.as_str());
                }
            }
        }
        for (_tx_hash, invs) in invocations {
            for inv in invs {
                if let Some(ref cid) = inv.contract_id {
                    contract_ids.insert(cid.as_str());
                }
            }
        }
        let cids: Vec<&str> = contract_ids.into_iter().collect();
        db::soroban::ensure_contracts_exist_batch(&mut **db_tx, &cids).await?;
    }

    // 4. Insert events — flatten all transactions into a single batch
    {
        let mut all_events = Vec::new();
        for (tx_hash, evts) in events {
            let Some(&tx_id) = hash_to_id.get(tx_hash.as_str()) else {
                warn!(tx_hash, "no transaction_id found for events — skipping");
                continue;
            };
            all_events.extend(evts.iter().map(|e| convert::to_event(e, tx_id)));
        }
        db::persistence::insert_events_batch(&mut **db_tx, &all_events).await?;
    }

    // 5. Insert invocations — flatten all transactions into a single batch
    {
        let mut all_invs = Vec::new();
        for (tx_hash, invs) in invocations {
            let Some(&tx_id) = hash_to_id.get(tx_hash.as_str()) else {
                warn!(
                    tx_hash,
                    "no transaction_id found for invocations — skipping"
                );
                continue;
            };
            all_invs.extend(invs.iter().map(|inv| convert::to_invocation(inv, tx_id)));
        }
        db::persistence::insert_invocations_batch(&mut **db_tx, &all_invs).await?;
    }

    // 6. Update operation trees — single batch UPDATE
    {
        let mut ids = Vec::with_capacity(operation_trees.len());
        let mut trees = Vec::with_capacity(operation_trees.len());
        for (tx_hash, tree) in operation_trees {
            let Some(&tx_id) = hash_to_id.get(tx_hash.as_str()) else {
                warn!(
                    tx_hash,
                    "no transaction_id found for operation_tree — skipping"
                );
                continue;
            };
            ids.push(tx_id);
            trees.push(tree);
        }
        db::soroban::update_operation_trees_batch(&mut **db_tx, &ids, &trees).await?;
    }

    // 7. Contract interface metadata (function signatures from WASM analysis).
    // TODO: ExtractedContractInterface only has wasm_hash, not contract_id.
    // We cannot store these rows correctly until we can join wasm_hash → contract_id
    // (e.g. via a wasm_hash index on soroban_contracts). Deferred to a follow-up task.
    let _ = contract_interfaces;

    // 8. Upsert contract deployments — merge duplicates (mirrors DB COALESCE logic)
    //    Required: PostgreSQL rejects duplicate keys in single INSERT...ON CONFLICT DO UPDATE
    {
        let mut merged: HashMap<&str, ExtractedContractDeployment> = HashMap::new();
        for d in contract_deployments {
            merged
                .entry(d.contract_id.as_str())
                .and_modify(|existing| {
                    // Mirror SQL: COALESCE(existing, new) — keep first non-null
                    if existing.wasm_hash.is_none() {
                        existing.wasm_hash.clone_from(&d.wasm_hash);
                    }
                    if existing.deployer_account.is_none() {
                        existing.deployer_account.clone_from(&d.deployer_account);
                    }
                    // is_sac = EXCLUDED.is_sac OR existing.is_sac
                    existing.is_sac = existing.is_sac || d.is_sac;
                    // metadata = existing || new (JSON merge)
                    if let serde_json::Value::Object(ref new_map) = d.metadata
                        && let serde_json::Value::Object(ref mut ex_map) = existing.metadata
                    {
                        for (k, v) in new_map {
                            ex_map.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                })
                .or_insert_with(|| d.clone());
        }
        let domain_contracts: Vec<_> = merged.values().map(convert::to_contract).collect();
        db::soroban::upsert_contract_deployments_batch(&mut **db_tx, &domain_contracts).await?;
    }

    // 9. Upsert account states — dedup by account_id, keep last
    {
        let mut deduped: HashMap<&str, _> = HashMap::new();
        for a in account_states {
            deduped.insert(a.account_id.as_str(), a);
        }
        let domain_accounts: Vec<_> = deduped.values().map(|a| convert::to_account(a)).collect();
        db::soroban::upsert_account_states_batch(&mut **db_tx, &domain_accounts).await?;
    }

    // 10. Upsert liquidity pools — dedup by pool_id, keep last
    {
        let mut deduped: HashMap<&str, _> = HashMap::new();
        for lp in liquidity_pools {
            deduped.insert(lp.pool_id.as_str(), lp);
        }
        let domain_pools: Vec<_> = deduped
            .values()
            .map(|lp| convert::to_liquidity_pool(lp))
            .collect();
        db::soroban::upsert_liquidity_pools_batch(&mut **db_tx, &domain_pools).await?;
    }

    // 11. Insert pool snapshots — dedup by (pool_id, ledger_sequence, created_at), keep last
    {
        let mut deduped: HashMap<(&str, u32, i64), _> = HashMap::new();
        for s in pool_snapshots {
            deduped.insert((s.pool_id.as_str(), s.ledger_sequence, s.created_at), s);
        }
        let domain_snapshots: Vec<_> = deduped
            .values()
            .map(|s| convert::to_pool_snapshot(s))
            .collect();
        db::soroban::insert_liquidity_pool_snapshots_batch(&mut **db_tx, &domain_snapshots).await?;
    }

    // 12. Upsert tokens — single batch (DO NOTHING, no dedup needed)
    {
        let domain_tokens: Vec<_> = tokens.iter().map(convert::to_token).collect();
        db::soroban::upsert_tokens_batch(&mut **db_tx, &domain_tokens).await?;
    }

    // 13. Upsert NFTs — merge duplicates (mirrors DB COALESCE logic)
    //    Required: PostgreSQL rejects duplicate keys in single INSERT...ON CONFLICT DO UPDATE
    {
        let mut merged: HashMap<(&str, &str), ExtractedNft> = HashMap::new();
        for n in nfts {
            merged
                .entry((n.contract_id.as_str(), n.token_id.as_str()))
                .and_modify(|existing| {
                    // Mirror SQL: owner_account = EXCLUDED (always overwrite)
                    existing.owner_account.clone_from(&n.owner_account);
                    // Mirror SQL: COALESCE(EXCLUDED.name, nfts.name) — prefer new if non-null
                    if n.name.is_some() {
                        existing.name.clone_from(&n.name);
                    }
                    if n.media_url.is_some() {
                        existing.media_url.clone_from(&n.media_url);
                    }
                    if n.metadata.is_some() {
                        existing.metadata.clone_from(&n.metadata);
                    }
                    // last_seen_ledger = EXCLUDED (always overwrite)
                    existing.last_seen_ledger = n.last_seen_ledger;
                    existing.created_at = n.created_at;
                })
                .or_insert_with(|| n.clone());
        }
        let domain_nfts: Vec<_> = merged.values().map(convert::to_nft).collect();
        db::soroban::upsert_nfts_batch(&mut **db_tx, &domain_nfts).await?;
    }

    Ok(())
}
