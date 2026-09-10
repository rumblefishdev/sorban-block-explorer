//! Task 0374 — one-shot fill of `liquidity_pools.legs` for classic pools that
//! still carry the empty default.
//!
//! ## Why this exists
//!
//! A pool's composition moved from the pair columns (`asset_a_*` / `asset_b_*`)
//! to a `legs` array, because a Soroban AMM pool has two to four legs and a
//! pair cannot hold three. The read path now reads `legs` and nothing else.
//!
//! `legs` is filled at WRITE time, and `liquidity_pools` is a
//! ReplacingMergeTree keyed on `pool_id` — one row per pool, replaced when the
//! pool is touched. So a pool gets its legs the next time the indexer writes
//! it, and **a pool that stopped trading is never written again**. Measured on
//! production 2026-09-09: 10,276 classic pools still empty, and not one of them
//! touched in the previous week. A ledger-range re-index cannot reach a pool
//! that did not trade in that range, however long it runs. This pass is the
//! only thing that clears the residue.
//!
//! ## Why it cannot be SQL
//!
//! The leg surrogate is our `cityhash_102_128` low half. ClickHouse's builtin
//! `cityHash64` is a DIFFERENT algorithm, so the value is not computable in the
//! database at all — it has to be produced by the same Rust function the writer
//! uses (`ids::pool_leg_asset_id`), which is exactly why this is a pass and not
//! an `ALTER … UPDATE`.
//!
//! ## Mechanism (mirrors `contract_type_rebuild` / `repair_tier1`)
//!
//! A plain re-INSERT would not do it: with equal RMT versions the surviving row
//! is undefined, and the version column here is `last_updated_ledger`, which is
//! real data that must not be bumped to force a win. So:
//!
//! 1. Read the still-empty pools and compute each one's legs in **Rust**.
//! 2. Push them to a temp verdict table for the join.
//! 3. Build a whole staging `liquidity_pools` via `INSERT … SELECT`, taking the
//!    verdict only where `legs` is empty; every other row and column passes
//!    through untouched.
//! 4. `EXCHANGE TABLES` (atomic) and drop the temps.
//!
//! Idempotent: a second run finds nothing empty and rebuilds an identical
//! table. `--dry-run` reports what it would fill and leaves the live table
//! alone.
//!
//! **Operational:**
//!
//! * Run with the indexer **STOPPED** — `EXCHANGE` swaps the whole table, so a
//!   concurrent live write between staging-build and swap is lost.
//! * Run it **BEFORE** the pair columns are dropped. It reads the very columns
//!   the migration removes; once they are gone the composition of an unfilled
//!   pool is not recoverable from the database at all.
//! * The deploy gate (`docs/deployment.md`) is
//!   `SELECT countIf(length(legs) = 0) FROM liquidity_pools FINAL` = 0. This
//!   pass is how that reaches zero.
//!
//! Delete this module once the pair columns are gone — it cannot compile
//! without them, and that is the point.

use clickhouse::Client as ClickhouseClient;
use clickhouse::Row;
use db_clickhouse::persist::ids;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::ch_staging::{create_staging_like, drop_if_exists, finalize};
use crate::error::BackfillError;
use crate::sink::Sink;

const VERDICT_TABLE: &str = "_pool_legs_verdict_0374";
const STAGING_TABLE: &str = "liquidity_pools_legs_staging_0374";

#[derive(Debug, Default, Clone, Copy)]
pub struct PoolLegsFillStats {
    /// Classic pools that carried an empty `legs` before the pass.
    pub unfilled_before: u64,
    /// Pools this pass produced legs for.
    pub filled: u64,
    /// Pools left empty because their pair columns say nothing either — real
    /// schema drift, and the one case worth looking at by hand.
    pub unresolvable: u64,
    pub dry_run: bool,
}

/// The pair columns of one pool that has no legs yet.
///
/// This struct exists because [`db_clickhouse::persist::rows::LiquidityPoolRow`]
/// no longer has these fields — the pass reads a schema the codebase has
/// already moved past, on purpose.
#[derive(Row, Deserialize)]
struct PairRow {
    pool_id: [u8; 32],
    asset_a_type: i16,
    asset_a_code: String,
    asset_a_issuer_id: i64,
    asset_b_type: i16,
    asset_b_code: String,
    asset_b_issuer_id: i64,
}

#[derive(Row, Serialize)]
struct VerdictRow {
    pool_id: [u8; 32],
    legs: Vec<i64>,
}

pub async fn execute(sink: &Sink, dry_run: bool) -> Result<PoolLegsFillStats, BackfillError> {
    let client = sink.client();
    let mut stats = PoolLegsFillStats {
        dry_run,
        ..Default::default()
    };

    // ---- Phase 1: compute the missing legs in Rust ----
    let pairs = read_unfilled(client).await?;
    stats.unfilled_before = pairs.len() as u64;
    info!(
        unfilled = stats.unfilled_before,
        "pool_legs_fill: classic pools with no legs"
    );

    let verdicts: Vec<VerdictRow> = pairs
        .into_iter()
        .filter_map(|p| {
            let legs = vec![
                ids::pool_leg_asset_id(p.asset_a_type, &p.asset_a_code, p.asset_a_issuer_id),
                ids::pool_leg_asset_id(p.asset_b_type, &p.asset_b_code, p.asset_b_issuer_id),
            ];
            // A surrogate of 0 is the "nothing is stored under this id" value:
            // the pair columns describe no asset, so there is no composition to
            // recover and writing one would invent it.
            if legs.iter().any(|&l| l == 0) {
                return None;
            }
            Some(VerdictRow {
                pool_id: p.pool_id,
                legs,
            })
        })
        .collect();
    stats.filled = verdicts.len() as u64;
    stats.unresolvable = stats.unfilled_before - stats.filled;

    if verdicts.is_empty() {
        info!("pool_legs_fill: nothing to fill — the gate is already open");
        return Ok(stats);
    }

    // ---- Phase 2: push the verdicts to a temp table for the join ----
    drop_if_exists(client, VERDICT_TABLE).await?;
    create_verdict_table(client, VERDICT_TABLE).await?;
    insert_verdicts(client, VERDICT_TABLE, &verdicts).await?;

    // ---- Phase 3: build the rebuilt table into staging ----
    drop_if_exists(client, STAGING_TABLE).await?;
    create_staging_like(client, "liquidity_pools", STAGING_TABLE).await?;
    build_staging(client, STAGING_TABLE, VERDICT_TABLE).await?;

    drop_if_exists(client, VERDICT_TABLE).await?;

    // ---- Phase 4: atomic swap (indexer STOPPED) ----
    finalize(client, "liquidity_pools", STAGING_TABLE, dry_run).await?;

    info!(
        unfilled_before = stats.unfilled_before,
        filled = stats.filled,
        unresolvable = stats.unresolvable,
        dry_run,
        "pool_legs_fill: completed"
    );
    Ok(stats)
}

/// The classic pools still carrying the empty default.
///
/// Soroban rows are excluded: their legs are token surrogates the pair columns
/// never described, so there is nothing here to recover them from. A Soroban
/// row with empty legs is a write-path defect, not a migration gap.
async fn read_unfilled(client: &ClickhouseClient) -> Result<Vec<PairRow>, BackfillError> {
    client
        .query(
            "SELECT pool_id, asset_a_type, asset_a_code, asset_a_issuer_id, \
                    asset_b_type, asset_b_code, asset_b_issuer_id \
             FROM liquidity_pools FINAL \
             WHERE length(legs) = 0 AND pool_kind = 0",
        )
        .fetch_all::<PairRow>()
        .await
        .map_err(BackfillError::Ch)
}

async fn create_verdict_table(client: &ClickhouseClient, table: &str) -> Result<(), BackfillError> {
    client
        .query(&format!(
            "CREATE TABLE {table} (pool_id FixedString(32), legs Array(Int64)) \
             ENGINE = MergeTree ORDER BY (pool_id)"
        ))
        .execute()
        .await
        .map_err(BackfillError::Ch)?;
    Ok(())
}

async fn insert_verdicts(
    client: &ClickhouseClient,
    table: &str,
    verdicts: &[VerdictRow],
) -> Result<(), BackfillError> {
    let mut insert = client
        .insert::<VerdictRow>(table)
        .await
        .map_err(BackfillError::Ch)?;
    for v in verdicts {
        insert.write(v).await.map_err(BackfillError::Ch)?;
    }
    insert.end().await.map_err(BackfillError::Ch)?;
    Ok(())
}

/// `SELECT lp.* REPLACE (…)` rather than an enumerated column list: this pass
/// runs against a schema mid-migration, and naming the columns would tie it to
/// one moment of that migration. The only column it touches is `legs`, and only
/// where it is empty — everything else, including the RMT version, passes
/// through byte for byte.
async fn build_staging(
    client: &ClickhouseClient,
    staging: &str,
    verdict: &str,
) -> Result<(), BackfillError> {
    client
        .query(&format!(
            "INSERT INTO {staging} \
             SELECT lp.* REPLACE (if(length(lp.legs) = 0, v.legs, lp.legs) AS legs) \
             FROM liquidity_pools lp FINAL \
             LEFT JOIN {verdict} v ON v.pool_id = lp.pool_id"
        ))
        .execute()
        .await
        .map_err(BackfillError::Ch)?;
    Ok(())
}
