//! Response DTO for `GET /v1/network/stats`.
//!
//! Wire shape per task 0045 + ADR 0021 §E1. The frontend consumes
//! this endpoint on every Home dashboard load — see
//! `docs/architecture/frontend/frontend-overview.md` §6.2.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Top-level chain overview returned by `GET /v1/network/stats`.
///
/// Field naming follows canonical SQL in task 0167
/// (`docs/architecture/database-schema/endpoint-queries/01_get_network_stats.sql`)
/// modulo one deliberate divergence: `ingestion_lag_seconds` is a
/// server-derived integer instead of canonical's raw
/// `latest_ledger_closed_at` timestamp. `total_accounts` and
/// `total_contracts` are planner estimates from `pg_class.reltuples`
/// (refreshed by autovacuum / ANALYZE), not exact counts.
/// `ingestion_lag_seconds` is `None` only on a cold-bootstrap cluster
/// where no ledger has been indexed yet.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NetworkStats {
    /// Transactions per second over a 60s rolling window. Computed as
    /// `SUM(ledgers.transaction_count)` divided by the actual span
    /// between MIN/MAX `closed_at` in the window. Stable on partial /
    /// single-ledger windows (NULLIF guards zero-span).
    pub tps_60s: f64,
    /// Estimated indexed account count from `pg_class.reltuples` for
    /// `public.accounts`.
    pub total_accounts: i64,
    /// Estimated indexed Soroban contract count from `pg_class.reltuples`
    /// for `public.soroban_contracts`.
    pub total_contracts: i64,
    /// Sequence of the newest ledger in the database (ordered by
    /// `closed_at DESC`). `0` sentinel indicates an empty cluster (no
    /// ledger 0 exists in Stellar).
    pub latest_ledger_sequence: i64,
    /// Seconds the indexer is behind the latest closed ledger's
    /// `closed_at`, computed server-side as
    /// `EXTRACT(EPOCH FROM now() - latest.closed_at)`. `null` only when
    /// no ledgers are indexed.
    pub ingestion_lag_seconds: Option<i64>,
}
