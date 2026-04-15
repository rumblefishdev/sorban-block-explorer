//! Persistence layer: writes all parsed data within a single DB transaction.

use std::collections::HashMap;
use std::time::Instant;
use tracing::{info, warn};

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
    let ledger_seq = ledger.sequence;
    macro_rules! timed {
        ($label:expr, $expr:expr) => {{
            let t = Instant::now();
            let result = $expr;
            (result, $label, t.elapsed())
        }};
    }
    let mut timings: Vec<(&str, std::time::Duration)> = Vec::with_capacity(16);

    // 1. Insert ledger
    let domain_ledger = convert::to_ledger(ledger);
    let (result, label, dur) = timed!(
        "insert_ledger",
        db::persistence::insert_ledger(&mut **db_tx, &domain_ledger).await
    );
    result?;
    timings.push((label, dur));

    // 2. Insert transactions and collect hash→id mapping
    let domain_txs: Vec<_> = transactions.iter().map(convert::to_transaction).collect();
    let (result, label, dur) = timed!(
        "insert_transactions",
        db::persistence::insert_transactions_batch(&mut **db_tx, &domain_txs).await
    );
    let tx_ids = result?;
    timings.push((label, dur));

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
        let (result, label, dur) = timed!(
            "insert_operations",
            db::persistence::insert_operations_batch(&mut **db_tx, &all_ops).await
        );
        result?;
        timings.push((label, dur));
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
        let (result, label, dur) = timed!(
            "ensure_contracts",
            db::soroban::ensure_contracts_exist_batch(&mut **db_tx, &cids).await
        );
        result?;
        timings.push((label, dur));
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
        let (result, label, dur) = timed!(
            "insert_events",
            db::persistence::insert_events_batch(&mut **db_tx, &all_events).await
        );
        result?;
        timings.push((label, dur));
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
        let (result, label, dur) = timed!(
            "insert_invocations",
            db::persistence::insert_invocations_batch(&mut **db_tx, &all_invs).await
        );
        result?;
        timings.push((label, dur));
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
        let (result, label, dur) = timed!(
            "update_op_trees",
            db::soroban::update_operation_trees_batch(&mut **db_tx, &ids, &trees).await
        );
        result?;
        timings.push((label, dur));
    }

    // 7. Upsert contract deployments — merge duplicates (mirrors DB COALESCE logic)
    //    Required: PostgreSQL rejects duplicate keys in single INSERT...ON CONFLICT DO UPDATE
    //    Must run BEFORE interface metadata so wasm_hash is populated.
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
        let (result, label, dur) = timed!(
            "upsert_contracts",
            db::soroban::upsert_contract_deployments_batch(&mut **db_tx, &domain_contracts).await
        );
        result?;
        timings.push((label, dur));
    }

    // 8. Contract interface metadata — dual-path persistence for the 2-ledger deploy pattern.
    //
    // Soroban separates WASM upload (ContractCodeEntry, ledger A) from contract deployment
    // (ContractDataEntry, ledger B). ExtractedContractInterface is only produced from
    // ContractCodeEntry, so by ledger B there is no interface data to apply directly.
    //
    // Strategy:
    //   a) Always upsert into wasm_interface_metadata (staging by wasm_hash) — covers ledger B.
    //   b) Also apply directly to any soroban_contracts rows that already exist — covers
    //      same-ledger deploys and re-index flows.
    //
    // upsert_contract_deployments_batch() applies wasm_interface_metadata after each batch upsert,
    // so any contract deployed in a later ledger automatically picks up the staged metadata.
    {
        let t = Instant::now();
        for iface in contract_interfaces {
            let metadata = serde_json::json!({
                "functions": iface.functions,
                "wasm_byte_len": iface.wasm_byte_len,
            });
            db::soroban::upsert_wasm_interface_metadata(&mut **db_tx, &iface.wasm_hash, &metadata)
                .await?;
            db::soroban::update_contract_interfaces_by_wasm_hash(
                &mut **db_tx,
                &iface.wasm_hash,
                &metadata,
            )
            .await?;
        }
        timings.push(("contract_interfaces", t.elapsed()));
    }

    // 9. Upsert account states — dedup + merge by account_id
    {
        let mut deduped: HashMap<&str, ExtractedAccountState> = HashMap::new();
        for a in account_states {
            match deduped.entry(a.account_id.as_str()) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(a.clone());
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    let existing = e.get_mut();
                    // sequence_number: non-sentinel wins (>= 0 over -1)
                    if a.sequence_number >= 0 {
                        existing.sequence_number = a.sequence_number;
                    }
                    if a.home_domain.is_some() {
                        existing.home_domain.clone_from(&a.home_domain);
                    }
                    if existing.first_seen_ledger.is_none() {
                        existing.first_seen_ledger = a.first_seen_ledger;
                    }
                    if a.last_seen_ledger > existing.last_seen_ledger {
                        existing.last_seen_ledger = a.last_seen_ledger;
                        existing.created_at = a.created_at;
                    }
                    // Merge balances: incoming entries override matching (by asset key)
                    if let (serde_json::Value::Array(ex_arr), serde_json::Value::Array(new_arr)) =
                        (&mut existing.balances, &a.balances)
                    {
                        for new_bal in new_arr {
                            let new_type = new_bal.get("asset_type");
                            let new_code = new_bal.get("asset_code");
                            let new_issuer = new_bal.get("issuer");
                            ex_arr.retain(|eb| {
                                eb.get("asset_type") != new_type
                                    || eb.get("asset_code") != new_code
                                    || eb.get("issuer") != new_issuer
                            });
                            ex_arr.push(new_bal.clone());
                        }
                    }
                    // Combine removed trustlines
                    existing
                        .removed_trustlines
                        .extend(a.removed_trustlines.iter().cloned());
                }
            }
        }

        // Collect removed trustlines before converting to domain objects
        let mut removals: Vec<(&str, &serde_json::Value)> = Vec::new();
        for a in deduped.values() {
            for rt in &a.removed_trustlines {
                removals.push((a.account_id.as_str(), rt));
            }
        }

        let domain_accounts: Vec<_> = deduped.values().map(|a| convert::to_account(a)).collect();
        let (result, label, dur) = timed!(
            "upsert_accounts",
            db::soroban::upsert_account_states_batch(&mut **db_tx, &domain_accounts).await
        );
        result?;
        timings.push((label, dur));

        // Remove deleted trustlines from DB (separate batch, skipped if empty)
        if !removals.is_empty() {
            let mut acct_ids = Vec::with_capacity(removals.len());
            let mut asset_types = Vec::with_capacity(removals.len());
            let mut asset_codes = Vec::with_capacity(removals.len());
            let mut issuers = Vec::with_capacity(removals.len());
            for (acct, rt) in &removals {
                acct_ids.push(*acct);
                asset_types.push(rt.get("asset_type").and_then(|v| v.as_str()).unwrap_or(""));
                asset_codes.push(rt.get("asset_code").and_then(|v| v.as_str()).unwrap_or(""));
                issuers.push(rt.get("issuer").and_then(|v| v.as_str()).unwrap_or(""));
            }
            let (result, label, dur) = timed!(
                "remove_trustlines",
                db::soroban::remove_trustlines_batch(
                    &mut **db_tx,
                    &acct_ids,
                    &asset_types,
                    &asset_codes,
                    &issuers,
                    ledger_seq as i64
                )
                .await
            );
            result?;
            timings.push((label, dur));
        }
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
        let (result, label, dur) = timed!(
            "upsert_pools",
            db::soroban::upsert_liquidity_pools_batch(&mut **db_tx, &domain_pools).await
        );
        result?;
        timings.push((label, dur));
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
        let (result, label, dur) = timed!(
            "insert_pool_snapshots",
            db::soroban::insert_liquidity_pool_snapshots_batch(&mut **db_tx, &domain_snapshots)
                .await
        );
        result?;
        timings.push((label, dur));
    }

    // 12. Upsert tokens — single batch (DO NOTHING, no dedup needed)
    {
        let domain_tokens: Vec<_> = tokens.iter().map(convert::to_token).collect();
        let (result, label, dur) = timed!(
            "upsert_tokens",
            db::soroban::upsert_tokens_batch(&mut **db_tx, &domain_tokens).await
        );
        result?;
        timings.push((label, dur));
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
        let (result, label, dur) = timed!(
            "upsert_nfts",
            db::soroban::upsert_nfts_batch(&mut **db_tx, &domain_nfts).await
        );
        result?;
        timings.push((label, dur));
    }

    // Log per-query breakdown
    let total: std::time::Duration = timings.iter().map(|(_, d)| *d).sum();
    let breakdown: Vec<String> = timings
        .iter()
        .map(|(label, dur)| format!("{}={:.1}ms", label, dur.as_secs_f64() * 1000.0))
        .collect();
    info!(
        ledger_seq,
        total_ms = format!("{:.1}", total.as_secs_f64() * 1000.0),
        "persist breakdown: {}",
        breakdown.join(" | ")
    );

    Ok(())
}
