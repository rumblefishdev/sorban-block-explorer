//! Liquidity-pool domain types matching the `liquidity_pools`,
//! `liquidity_pool_snapshots`, and `lp_positions` PostgreSQL tables.
//!
//! Schema: ADR 0027 Part I §14, §15, §16. Pool identity + asset pair is
//! fully typed (no JSONB); reserves live in the partitioned snapshots table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Pool identity + asset pair + fee (ADR 0027 §14).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityPool {
    /// 32 B pool hash.
    pub pool_id: Vec<u8>,
    pub asset_a_type: String,
    pub asset_a_code: Option<String>,
    pub asset_a_issuer_id: Option<i64>,
    pub asset_b_type: String,
    pub asset_b_code: Option<String>,
    pub asset_b_issuer_id: Option<i64>,
    pub fee_bps: i32,
    pub created_at_ledger: i64,
}

/// Per-ledger pool state snapshot (ADR 0027 §15). Partitioned on `created_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityPoolSnapshot {
    pub id: i64,
    pub pool_id: Vec<u8>,
    pub ledger_sequence: i64,
    /// NUMERIC(28,7) as decimal string.
    pub reserve_a: String,
    pub reserve_b: String,
    pub total_shares: String,
    pub tvl: Option<String>,
    pub volume: Option<String>,
    pub fee_revenue: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// LP participant / shares row (ADR 0027 §16).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LpPosition {
    pub pool_id: Vec<u8>,
    pub account_id: i64,
    /// NUMERIC(28,7) as decimal string.
    pub shares: String,
    pub first_deposit_ledger: i64,
    pub last_updated_ledger: i64,
}
