//! Database queries backing `GET /v1/network/stats`.
//!
//! All four queries are individually cheap and run sequentially on the
//! same pool connection — total miss latency dominated by the largest
//! `count(*)` (accounts table). Repeat-call traffic is absorbed by the
//! 30s in-memory cache (see `cache.rs`); the DB sees one round of these
//! queries every ~30s per warm Lambda instance.

use sqlx::{PgPool, Row};

use super::dto::NetworkStats;

/// Run the four network-stats queries against `pool` and assemble the
/// response DTO.
///
/// Returns `sqlx::Error` on any DB failure; the handler turns this into
/// the canonical `db_error` envelope. Queries are written in raw SQL
/// (not the `query!` macro) so they do not require `DATABASE_URL` at
/// build time — consistent with how `crates/api/src/transactions/queries.rs`
/// handles the same trade-off.
pub async fn fetch_stats(pool: &PgPool) -> Result<NetworkStats, sqlx::Error> {
    // One row from `ledgers` carries both the highest indexed sequence
    // and the lag derived from the latest `closed_at`. Combining them
    // into a single SELECT saves one round-trip on the cache-miss path.
    //
    // `coalesce(..., 0)` returns 0 when the table is empty (Stellar
    // genesis is ledger 1, so 0 is a safe sentinel for "no data").
    // The `CASE` keeps `ingestion_lag_seconds` nullable distinctly from
    // the sequence, since lag is only computable once we have data.
    let ledger_row = sqlx::query(
        "SELECT \
            COALESCE(max(sequence), 0)::BIGINT AS highest_indexed_ledger, \
            CASE WHEN max(closed_at) IS NULL THEN NULL \
                 ELSE EXTRACT(EPOCH FROM now() - max(closed_at))::BIGINT \
            END AS ingestion_lag_seconds \
         FROM ledgers",
    )
    .fetch_one(pool)
    .await?;

    let highest_indexed_ledger: i64 = ledger_row.get("highest_indexed_ledger");
    let ingestion_lag_seconds: Option<i64> = ledger_row.get("ingestion_lag_seconds");

    // TPS — 60s rolling window per ADR 0021 §E1. Source is the
    // `transactions` partitioned fact table; the recent `created_at`
    // predicate is expected to stay within the newest partition(s) via
    // partition pruning.
    // `::float8` cast yields f64 server-side so we don't materialise a
    // PostgreSQL `NUMERIC` (which would need rust_decimal to decode).
    let tps: f64 = sqlx::query_scalar(
        "SELECT count(*)::float8 / 60.0 \
         FROM transactions \
         WHERE created_at > now() - interval '1 minute'",
    )
    .fetch_one(pool)
    .await?;

    let total_accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts")
        .fetch_one(pool)
        .await?;

    let total_contracts: i64 = sqlx::query_scalar("SELECT count(*) FROM soroban_contracts")
        .fetch_one(pool)
        .await?;

    Ok(NetworkStats {
        tps,
        total_accounts,
        total_contracts,
        highest_indexed_ledger,
        ingestion_lag_seconds,
    })
}
