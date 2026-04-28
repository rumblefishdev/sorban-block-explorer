//! Database queries backing `GET /v1/network/stats`.
//!
//! Implementation of the canonical SQL in
//! `docs/architecture/database-schema/endpoint-queries/01_get_network_stats.sql`
//! (deliverable of task 0167) — one statement, one round-trip.
//!
//! Cost profile per cache miss:
//!   * latest ledger:    index lookup on `idx_ledgers_closed_at DESC`, 1 row.
//!   * tps_60s:          index range on the same index, ~12 rows (5s × 60s).
//!   * total_accounts:   `pg_class.reltuples` catalog read, microseconds.
//!   * total_contracts:  `pg_class.reltuples` catalog read, microseconds.
//!
//! No `count(*)` over user tables — at chain scale (10s of millions of
//! accounts) an exact count is a full heap scan that would dominate this
//! hot dashboard path. Reltuples is refreshed by autovacuum / ANALYZE
//! and is well within the accuracy a "total accounts" UI cell needs.
//!
//! Repeat-call traffic is absorbed by the 30s in-process cache (see
//! `cache.rs`); the DB sees one statement every ~30s per warm Lambda.

use sqlx::{PgPool, Row};

use super::dto::NetworkStats;

/// Run the canonical network-stats statement against `pool` and assemble
/// the response DTO.
///
/// Returns `sqlx::Error` on any DB failure; the handler turns this into
/// the canonical `db_error` envelope. Written in raw SQL (not the
/// `query!` macro) so it does not require `DATABASE_URL` at build time
/// — consistent with `crates/api/src/transactions/queries.rs`.
///
/// Empty-`ledgers` case (cold-bootstrap cluster, no rows ingested yet):
/// the canonical SELECT yields zero rows because the inner
/// `ORDER BY closed_at DESC LIMIT 1` is empty. We map that to a
/// zero-valued response with `ingestion_lag_seconds = None`, preserving
/// the wire shape from before the canonical-SQL alignment.
///
/// DTO field naming intentionally retained (`tps`, `highest_indexed_ledger`,
/// `ingestion_lag_seconds`) pending the PM call flagged in the PR review:
/// the canonical SQL exposes `tps_60s`, `latest_ledger_sequence`, and a
/// raw `latest_ledger_closed_at` timestamp instead of a derived lag. Lag
/// is computed inline via `EXTRACT(EPOCH FROM now() - latest.closed_at)`
/// so the wire contract is unchanged for now.
pub async fn fetch_stats(pool: &PgPool) -> Result<NetworkStats, sqlx::Error> {
    let row_opt = sqlx::query(
        "SELECT \
            latest.sequence AS highest_indexed_ledger, \
            EXTRACT(EPOCH FROM now() - latest.closed_at)::BIGINT AS ingestion_lag_seconds, \
            ( \
                SELECT COALESCE( \
                    SUM(transaction_count)::float8 \
                        / NULLIF(EXTRACT(EPOCH FROM (MAX(closed_at) - MIN(closed_at))), 0), \
                    0 \
                )::float8 \
                FROM ledgers \
                WHERE closed_at >= now() - INTERVAL '60 seconds' \
            ) AS tps, \
            (SELECT GREATEST(reltuples::bigint, 0) FROM pg_class \
                WHERE oid = 'public.accounts'::regclass) AS total_accounts, \
            (SELECT GREATEST(reltuples::bigint, 0) FROM pg_class \
                WHERE oid = 'public.soroban_contracts'::regclass) AS total_contracts \
         FROM ( \
             SELECT sequence, closed_at \
             FROM ledgers \
             ORDER BY closed_at DESC \
             LIMIT 1 \
         ) latest",
    )
    .fetch_optional(pool)
    .await?;

    let Some(row) = row_opt else {
        // Empty `ledgers` table — cold-bootstrap cluster. Sequence 0 is a
        // safe sentinel (Stellar genesis is ledger 1) and lag is undefined
        // when no ledger has been ingested.
        return Ok(NetworkStats {
            tps: 0.0,
            total_accounts: 0,
            total_contracts: 0,
            highest_indexed_ledger: 0,
            ingestion_lag_seconds: None,
        });
    };

    Ok(NetworkStats {
        tps: row.get("tps"),
        total_accounts: row.get("total_accounts"),
        total_contracts: row.get("total_contracts"),
        highest_indexed_ledger: row.get("highest_indexed_ledger"),
        ingestion_lag_seconds: row.get("ingestion_lag_seconds"),
    })
}
