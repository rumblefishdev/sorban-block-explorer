//! ADR 0027 write-path — one atomic DB transaction per ledger.
//!
//! Pipeline order matches the FK dependency graph (note the `operations.pool_id`
//! FK added in migration 0006 forces `liquidity_pools` to land before `operations`,
//! so the upsert reorders vs. the per-task narrative):
//!
//!   1. accounts             (StrKey → id map built here)
//!   2. wasm_interface_metadata
//!   3. soroban_contracts
//!   4. ledgers
//!   5. transactions         (tx_hash → id map built here)
//!   6. transaction_hash_index
//!   7. transaction_participants
//!   8. liquidity_pools + liquidity_pool_snapshots + lp_positions
//!   9. operations           (FK → liquidity_pools.pool_id)
//!  10. soroban_events_appearances       (ADR 0033 — aggregate index per trio)
//!  11. soroban_invocations_appearances  (ADR 0034 — aggregate index per trio + caller payload)
//!  12. assets
//!  13. nfts + nft_ownership
//!  14. account_balances_current + account_balance_history + trustline deletes
//!
//! Every write uses UNNEST batching; pool.begin()/commit() is retried with
//! exponential backoff on SQLSTATE 40001 (serialization) / 40P01 (deadlock).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use sqlx::PgPool;
use tracing::{info, warn};
use xdr_parser::types::{
    ExtractedAccountState, ExtractedAsset, ExtractedContractDeployment, ExtractedContractInterface,
    ExtractedEvent, ExtractedInvocation, ExtractedLedger, ExtractedLiquidityPool,
    ExtractedLiquidityPoolSnapshot, ExtractedLpPosition, ExtractedNft, ExtractedNftEvent,
    ExtractedOperation, ExtractedTransaction,
};

use super::HandlerError;

mod classification_cache;
mod staging;
mod write;

pub use classification_cache::ClassificationCache;
use staging::Staged;

/// Max retries on transient serialization / deadlock errors (40001 / 40P01).
const RETRY_BACKOFF_MS: [u64; 3] = [50, 200, 800];

/// Per-step timings captured inside the DB transaction.
#[derive(Default, Debug)]
struct StepTimings {
    accounts_ms: u128,
    wasm_ms: u128,
    contracts_ms: u128,
    ledgers_ms: u128,
    transactions_ms: u128,
    hash_index_ms: u128,
    participants_ms: u128,
    operations_ms: u128,
    events_ms: u128,
    invocations_ms: u128,
    assets_ms: u128,
    nfts_ms: u128,
    pools_ms: u128,
    balances_ms: u128,
    stage_ms: u128,
}

/// Persist all parsed data for a single ledger into the ADR 0027 schema.
///
/// Owns the transaction lifecycle: opens it, runs all 14 write steps inside
/// it, commits on success, and retries the whole envelope on serialization
/// failures. The caller passes the connection pool (not a `Transaction`) so
/// a retry can start a fresh tx cleanly.
///
/// Signature parameters that the parser does not yet populate:
///
/// * `nft_events`        — `nft_ownership` rows (task 0118 / follow-up)
/// * `lp_positions`      — `lp_positions` rows (task 0126)
/// * `inner_tx_hashes`   — `transactions.inner_tx_hash` (follow-up parser work)
///
/// `process_ledger` passes empty slices / `None` for these until the parser
/// catches up; the wiring is already in place end-to-end.
#[allow(clippy::too_many_arguments)]
pub async fn persist_ledger(
    pool: &PgPool,
    ledger: &ExtractedLedger,
    transactions: &[ExtractedTransaction],
    operations: &[(String, Vec<ExtractedOperation>)],
    events: &[(String, Vec<ExtractedEvent>)],
    invocations: &[(String, Vec<ExtractedInvocation>)],
    _operation_trees: &[(String, serde_json::Value)],
    contract_interfaces: &[ExtractedContractInterface],
    contract_deployments: &[ExtractedContractDeployment],
    account_states: &[ExtractedAccountState],
    liquidity_pools: &[ExtractedLiquidityPool],
    pool_snapshots: &[ExtractedLiquidityPoolSnapshot],
    assets: &[ExtractedAsset],
    nfts: &[ExtractedNft],
    nft_events: &[ExtractedNftEvent],
    lp_positions: &[ExtractedLpPosition],
    inner_tx_hashes: &HashMap<String, Option<String>>,
    classification_cache: &ClassificationCache,
) -> Result<(), HandlerError> {
    let stage_start = Instant::now();
    let staged = Staged::prepare(
        ledger,
        transactions,
        operations,
        events,
        invocations,
        contract_interfaces,
        contract_deployments,
        account_states,
        liquidity_pools,
        pool_snapshots,
        assets,
        nfts,
        nft_events,
        lp_positions,
        inner_tx_hashes,
    )?;
    let stage_ms = stage_start.elapsed().as_millis();

    let mut attempt = 0usize;
    let timings = loop {
        let mut db_tx = pool.begin().await?;
        let mut timings = StepTimings {
            stage_ms,
            ..StepTimings::default()
        };

        match run_all_steps(&mut db_tx, &staged, &mut timings, classification_cache).await {
            Ok(()) => match db_tx.commit().await {
                Ok(()) => break timings,
                Err(err) => {
                    if let Some(delay) = retry_delay(&err, attempt) {
                        warn!(
                            ledger_sequence = ledger.sequence,
                            attempt,
                            backoff_ms = delay.as_millis() as u64,
                            error = %err,
                            "commit hit transient conflict — retrying"
                        );
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(err.into());
                }
            },
            Err(err) => {
                // Rollback is implicit on drop, but be explicit so a failing
                // rollback doesn't mask the original error.
                let _ = db_tx.rollback().await;
                if let HandlerError::Database(ref db_err) = err
                    && let Some(delay) = retry_delay(db_err, attempt)
                {
                    warn!(
                        ledger_sequence = ledger.sequence,
                        attempt,
                        backoff_ms = delay.as_millis() as u64,
                        error = %db_err,
                        "persist hit transient conflict — retrying"
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    continue;
                }
                return Err(err);
            }
        }
    };

    let total_ms = timings.stage_ms
        + timings.accounts_ms
        + timings.wasm_ms
        + timings.contracts_ms
        + timings.ledgers_ms
        + timings.transactions_ms
        + timings.hash_index_ms
        + timings.participants_ms
        + timings.operations_ms
        + timings.events_ms
        + timings.invocations_ms
        + timings.assets_ms
        + timings.nfts_ms
        + timings.pools_ms
        + timings.balances_ms;

    info!(
        ledger_sequence = ledger.sequence,
        total_ms,
        stage_ms = timings.stage_ms,
        accounts_ms = timings.accounts_ms,
        wasm_ms = timings.wasm_ms,
        contracts_ms = timings.contracts_ms,
        ledgers_ms = timings.ledgers_ms,
        transactions_ms = timings.transactions_ms,
        hash_index_ms = timings.hash_index_ms,
        participants_ms = timings.participants_ms,
        operations_ms = timings.operations_ms,
        events_ms = timings.events_ms,
        invocations_ms = timings.invocations_ms,
        assets_ms = timings.assets_ms,
        nfts_ms = timings.nfts_ms,
        pools_ms = timings.pools_ms,
        balances_ms = timings.balances_ms,
        retries = attempt,
        "persist breakdown"
    );

    Ok(())
}

/// Drive all 14 DB steps inside the open transaction, recording per-step timings.
async fn run_all_steps(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    staged: &Staged,
    timings: &mut StepTimings,
    classification_cache: &ClassificationCache,
) -> Result<(), HandlerError> {
    let ledger_sequence = staged.ledger_sequence;

    // Async commit — skip the per-commit fsync wait. Safe for this indexer
    // because the S3 event source + Lambda retry policy provides end-to-end
    // durability: a crash between COMMIT and fsync simply re-delivers the
    // ledger event and we re-ingest idempotently. Saves the commit-fsync
    // latency (~5–15 ms on typical disks) from every ledger.
    sqlx::query("SET LOCAL synchronous_commit = off")
        .execute(&mut **db_tx)
        .await?;

    let t = Instant::now();
    let account_ids = write::upsert_accounts(db_tx, staged).await?;
    timings.accounts_ms = t.elapsed().as_millis();

    let t = Instant::now();
    write::upsert_wasm_metadata(db_tx, staged).await?;
    write::stub_unknown_wasm_interfaces(db_tx, staged).await?;
    // Task 0118 Phase 2 — apply classification from wasm specs processed
    // this ledger to any `soroban_contracts` row sharing the wasm_hash.
    // Runs inside the same tx so reclassification is visible to the NFT
    // filter step further down.
    write::reclassify_contracts_from_wasm(db_tx, staged).await?;
    timings.wasm_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let contract_ids = write::upsert_contracts_returning_id(db_tx, staged, &account_ids).await?;
    timings.contracts_ms = t.elapsed().as_millis();
    // Populate per-worker cache with definitive classifications observed
    // this ledger (SAC → Token, WASM-classified deployments → Nft/Fungible).
    // Never cache `Other` — a later WASM upload can still promote it.
    write::populate_cache_from_staged(staged, classification_cache);

    let t = Instant::now();
    write::insert_ledger(db_tx, staged).await?;
    timings.ledgers_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let tx_ids = write::insert_transactions(db_tx, staged, &account_ids).await?;
    timings.transactions_ms = t.elapsed().as_millis();

    let t = Instant::now();
    write::insert_hash_index(db_tx, staged).await?;
    timings.hash_index_ms = t.elapsed().as_millis();

    let t = Instant::now();
    write::insert_participants(db_tx, staged, &account_ids, &tx_ids).await?;
    timings.participants_ms = t.elapsed().as_millis();

    let t = Instant::now();
    write::upsert_pools_and_snapshots(db_tx, staged, &account_ids).await?;
    timings.pools_ms = t.elapsed().as_millis();

    let t = Instant::now();
    write::insert_operations(db_tx, staged, &account_ids, &contract_ids, &tx_ids).await?;
    timings.operations_ms = t.elapsed().as_millis();

    let t = Instant::now();
    write::insert_events(db_tx, staged, &contract_ids, &tx_ids).await?;
    timings.events_ms = t.elapsed().as_millis();

    let t = Instant::now();
    write::insert_invocations(db_tx, staged, &account_ids, &contract_ids, &tx_ids).await?;
    timings.invocations_ms = t.elapsed().as_millis();

    let t = Instant::now();
    write::upsert_assets(db_tx, staged, &account_ids, &contract_ids).await?;
    // Task 0120 — bridge late-WASM reclassification into the `assets` table.
    // Handles the two-ledger deploy pattern (contract deployed earlier,
    // WASM uploaded now) that `detect_assets` cannot observe in-memory.
    write::insert_assets_from_reclassified_contracts(db_tx, staged).await?;
    timings.assets_ms = t.elapsed().as_millis();

    let t = Instant::now();
    // Task 0118 Phase 2 — `upsert_nfts_and_ownership` is responsible for
    // hydrating the per-worker cache for any NFT-candidate contracts
    // unseen so far and dropping rows whose contract is classified as
    // `Token` or `Fungible`. Runs inside this tx so the batch SELECT sees
    // the reclassification UPDATE we applied after the wasm upsert.
    write::upsert_nfts_and_ownership(
        db_tx,
        staged,
        &account_ids,
        &contract_ids,
        &tx_ids,
        classification_cache,
    )
    .await?;
    timings.nfts_ms = t.elapsed().as_millis();

    let t = Instant::now();
    write::upsert_balances(db_tx, staged, &account_ids).await?;
    timings.balances_ms = t.elapsed().as_millis();

    let _ = ledger_sequence;
    Ok(())
}

/// Return `Some(backoff)` if `err` is a retryable PG conflict and we still
/// have attempts left; otherwise `None`.
fn retry_delay(err: &sqlx::Error, attempt: usize) -> Option<Duration> {
    if attempt >= RETRY_BACKOFF_MS.len() {
        return None;
    }
    let code = match err {
        sqlx::Error::Database(db_err) => db_err.code()?,
        _ => return None,
    };
    if code == "40001" || code == "40P01" {
        Some(Duration::from_millis(RETRY_BACKOFF_MS[attempt]))
    } else {
        None
    }
}
