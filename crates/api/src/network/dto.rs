//! Response DTO for `GET /v1/network/stats`.
//!
//! Wire shape per task 0045 + ADR 0021 §E1. The frontend consumes
//! this endpoint on every Home dashboard load — see
//! `docs/architecture/frontend/frontend-overview.md` §6.2.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Top-level chain overview returned by `GET /v1/network/stats`.
///
/// All counts are exact (no estimation). `ingestion_lag_seconds` is
/// `None` only on a cold-bootstrap cluster where no ledger has been
/// indexed yet.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NetworkStats {
    /// Current transactions per second — 60s rolling window over
    /// `transactions.created_at` per ADR 0021 §E1. Float because the
    /// formula is `count / 60.0`; consumers display rounded.
    pub tps: f64,
    /// Total indexed account count — `SELECT count(*) FROM accounts`.
    pub total_accounts: i64,
    /// Total indexed Soroban contract count —
    /// `SELECT count(*) FROM soroban_contracts`.
    pub total_contracts: i64,
    /// Highest ledger sequence currently in the database. Equal to
    /// `coalesce(max(sequence), 0)` from `ledgers`; the `0` sentinel
    /// indicates an empty cluster (no ledger 0 exists in Stellar).
    pub highest_indexed_ledger: i64,
    /// Estimated seconds the indexer is behind the latest closed
    /// ledger's `closed_at`. `null` only when no ledgers are indexed.
    pub ingestion_lag_seconds: Option<i64>,
}
