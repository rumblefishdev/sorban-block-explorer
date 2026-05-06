//! Topological merge — orchestrates the 15 substeps from task 0186 §Step 3
//! / ADR 0040 "Full topological merge order".
//!
//! **Each table has its own file.** Schema changes for table X mean
//! editing exactly one file under `steps/`. The order below is FK-graph
//! verified — do not reorder without re-checking ADR 0040 §"Full
//! topological merge order".
//!
//! Phase D3 (current): 5 appearance tables (FK rewrite via JOIN
//! `merge_remap.*`) + 2 watermark tables (`lp_positions`,
//! `account_balances_current`). All 18 ADR-0040 table-by-table semantics
//! are now implemented; only `finalize` (Phase D4) remains.

use sqlx::{Executor, PgConnection};

use crate::error::MergeError;

pub mod account_balances_current;
pub mod accounts;
pub mod assets;
pub mod ledgers;
pub mod liquidity_pool_snapshots;
pub mod liquidity_pools;
pub mod lp_positions;
pub mod nft_ownership;
pub mod nfts;
pub mod operations_appearances;
pub mod soroban_contracts;
pub mod soroban_events_appearances;
pub mod soroban_invocations_appearances;
pub mod transaction_hash_index;
pub mod transaction_participants;
pub mod transactions;
pub mod wasm_interface_metadata;

/// Wrap one step in a per-table transaction (per task 0186 §Step 0
/// atomicity decision: tx-per-table, SAVEPOINTs inside). A macro instead
/// of a generic fn because async-fn-as-generic-arg loses lifetime info
/// across the HRTB and rustc can't satisfy `FnOnce(&mut PgConnection)`.
macro_rules! step {
    ($conn:expr, $module:ident) => {{
        let table = stringify!($module);
        $conn.execute("BEGIN").await?;
        match $module::run($conn).await {
            Ok(stats) => {
                $conn.execute("COMMIT").await?;
                tracing::info!(
                    table,
                    batches = stats.batches,
                    rows = stats.rows_affected,
                    "step complete"
                );
            }
            Err(e) => {
                $conn.execute("ROLLBACK").await?;
                tracing::error!(table, error = %e, "step failed — table rolled back");
                return Err(e);
            }
        }
    }};
}

pub async fn execute(conn: &mut PgConnection) -> Result<(), MergeError> {
    setup_workspace(conn).await?;

    // Order is FK-graph topological per ADR 0040 §"Full topological
    // merge order". Each REMAP step populates the corresponding
    // `merge_remap` table so subsequent steps can JOIN through it.
    step!(conn, ledgers);
    step!(conn, accounts); // REMAP
    step!(conn, wasm_interface_metadata);
    step!(conn, soroban_contracts); // REMAP (FK deployer_id → accounts)
    step!(conn, assets); // FK issuer_id, contract_id
    step!(conn, liquidity_pools); // FK asset_a/b_issuer_id → accounts
    step!(conn, nfts); // REMAP (FK contract_id → soroban_contracts)
    step!(conn, transactions); // REMAP (FK source_id → accounts)
    step!(conn, transaction_hash_index);
    // 5 appearance tables — FK rewrite via JOIN merge_remap.*
    step!(conn, operations_appearances);
    step!(conn, transaction_participants);
    step!(conn, soroban_events_appearances);
    step!(conn, soroban_invocations_appearances);
    step!(conn, nft_ownership);
    step!(conn, liquidity_pool_snapshots);
    // 2 watermark tables — last_updated_ledger guarded UPSERT
    step!(conn, lp_positions);
    step!(conn, account_balances_current);

    teardown_workspace(conn).await?;
    Ok(())
}

/// `merge_remap` schema holds the per-table source_id → target_id maps
/// captured during the four REMAP passes. UNLOGGED — these are transient
/// (dropped at end of merge) so WAL overhead is wasted. PRIMARY KEY on
/// `source_id` doubles as the B-tree index that D3's appearance-table
/// FK rewrites JOIN against (per ADR 0040: "Build B-tree index on
/// `merge_remap.<parent>(source_id)` before the JOINs — without it,
/// 150M-row JOINs do nested loops and never finish.").
async fn setup_workspace(conn: &mut PgConnection) -> Result<(), MergeError> {
    conn.execute("DROP SCHEMA IF EXISTS merge_remap CASCADE")
        .await?;
    conn.execute("CREATE SCHEMA merge_remap").await?;
    conn.execute(
        "CREATE UNLOGGED TABLE merge_remap.accounts (
            source_id BIGINT PRIMARY KEY,
            target_id BIGINT NOT NULL
         )",
    )
    .await?;
    conn.execute(
        "CREATE UNLOGGED TABLE merge_remap.soroban_contracts (
            source_id BIGINT PRIMARY KEY,
            target_id BIGINT NOT NULL
         )",
    )
    .await?;
    // nfts.id is SERIAL (INTEGER), not BIGSERIAL — match types so the
    // JOIN doesn't silently widen and disable index usage.
    conn.execute(
        "CREATE UNLOGGED TABLE merge_remap.nfts (
            source_id INTEGER PRIMARY KEY,
            target_id INTEGER NOT NULL
         )",
    )
    .await?;
    // transactions remap carries created_at because partition routing on
    // appearance-table FKs needs it (composite FK on (transaction_id,
    // created_at)). Source and target created_at are the same value
    // (dedup is by (hash, created_at)) so one column suffices.
    conn.execute(
        "CREATE UNLOGGED TABLE merge_remap.transactions (
            source_id BIGINT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            target_id BIGINT NOT NULL,
            PRIMARY KEY (source_id, created_at)
         )",
    )
    .await?;
    Ok(())
}

async fn teardown_workspace(conn: &mut PgConnection) -> Result<(), MergeError> {
    conn.execute("DROP SCHEMA IF EXISTS merge_remap CASCADE")
        .await?;
    Ok(())
}
