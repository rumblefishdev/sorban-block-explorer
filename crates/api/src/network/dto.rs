//! Response DTO for `GET /v1/network/stats`.
//!
//! Wire shape per task 0045 + ADR 0021 §E1. The frontend consumes
//! this endpoint on every Home dashboard load — see
//! `docs/architecture/frontend/frontend-overview.md` §6.2.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Top-level chain overview returned by `GET /v1/network/stats`.
///
/// `total_accounts` and `total_contracts` are planner estimates from
/// `pg_class.reltuples` (refreshed by autovacuum / ANALYZE), not exact
/// counts — see `queries.rs` for the rationale. `ingestion_lag_seconds`
/// is `None` only on a cold-bootstrap cluster where no ledger has been
/// indexed yet.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NetworkStats {
    /// Current transactions per second — 60s rolling window computed
    /// from `SUM(ledgers.transaction_count)` divided by the actual
    /// span between MIN/MAX `closed_at` in the window (per ADR 0021
    /// §E1 and the canonical SQL in task 0167). Yields a stable rate
    /// even on partial / single-ledger windows; consumers display
    /// rounded.
    pub tps: f64,
    /// Estimated indexed account count from `pg_class.reltuples` for
    /// `public.accounts`. Estimate (not exact) — see `queries.rs`.
    pub total_accounts: i64,
    /// Estimated indexed Soroban contract count from `pg_class.reltuples`
    /// for `public.soroban_contracts`. Estimate (not exact).
    pub total_contracts: i64,
    /// Highest ledger sequence currently in the database. Sourced from
    /// the newest row in `ledgers` ordered by `closed_at DESC`. `0`
    /// sentinel indicates an empty cluster (no ledger 0 exists in
    /// Stellar).
    pub highest_indexed_ledger: i64,
    /// Seconds the indexer is behind the latest closed ledger's
    /// `closed_at`, computed server-side as
    /// `EXTRACT(EPOCH FROM now() - latest.closed_at)`. `null` only when
    /// no ledgers are indexed.
    pub ingestion_lag_seconds: Option<i64>,
}
