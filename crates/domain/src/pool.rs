//! Liquidity pool domain types matching the `liquidity_pools` and
//! `liquidity_pool_snapshots` PostgreSQL tables.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Liquidity pool record as stored in PostgreSQL.
///
/// Unpartitioned current-state entity. Updated via watermark-guarded upserts
/// (`last_updated_ledger`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityPool {
    /// Pool hash identifier (64 chars). Primary key.
    pub pool_id: String,
    /// First reserve asset as JSONB.
    pub asset_a: serde_json::Value,
    /// Second reserve asset as JSONB.
    pub asset_b: serde_json::Value,
    /// Fee in basis points.
    pub fee_bps: i32,
    /// Current reserves as JSONB.
    pub reserves: serde_json::Value,
    /// Total pool share tokens outstanding (NUMERIC as string).
    pub total_shares: String,
    /// Total value locked (NUMERIC as string).
    pub tvl: Option<String>,
    /// Ledger at which the pool was created (FK to ledgers.sequence).
    pub created_at_ledger: i64,
    /// Most recent ledger with pool state change. Watermark.
    pub last_updated_ledger: i64,
}

/// Liquidity pool snapshot as stored in PostgreSQL.
///
/// Append-only time-series table, partitioned by `created_at`.
/// Composite PK: `(id, created_at)`. Unique on `(pool_id, ledger_sequence, created_at)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityPoolSnapshot {
    /// Surrogate primary key (BIGSERIAL).
    pub id: i64,
    /// Parent pool (FK to liquidity_pools.pool_id).
    pub pool_id: String,
    /// Ledger sequence at snapshot time.
    pub ledger_sequence: i64,
    /// Snapshot timestamp for partitioning.
    pub created_at: DateTime<Utc>,
    /// Reserves at snapshot time as JSONB.
    pub reserves: serde_json::Value,
    /// Total pool shares at snapshot time (NUMERIC as string).
    pub total_shares: String,
    /// Total value locked at snapshot time (NUMERIC as string).
    pub tvl: Option<String>,
    /// Trading volume in the snapshot period (NUMERIC as string).
    pub volume: Option<String>,
    /// Fee revenue in the snapshot period (NUMERIC as string).
    pub fee_revenue: Option<String>,
}
