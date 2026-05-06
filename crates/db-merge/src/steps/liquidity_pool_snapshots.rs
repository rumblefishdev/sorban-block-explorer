//! `liquidity_pool_snapshots` (partitioned) — dedup-only via
//! `uq_lp_snapshots_pool_ledger`; no FK referrers. Mirror of
//! `write.rs:1702`.
//!
//! Partitioned table — `ledger_sequence`-windowed batching also gives
//! Postgres constraint exclusion (only the `*_default` partition exists
//! today, but if monthly children appear later, the WHERE clause still
//! prunes correctly).

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "liquidity_pool_snapshots",
        "merge_source.liquidity_pool_snapshots",
        "ledger_sequence",
        r#"
        INSERT INTO liquidity_pool_snapshots (
            pool_id, ledger_sequence, reserve_a, reserve_b, total_shares,
            tvl, volume, fee_revenue, created_at
        )
        SELECT pool_id, ledger_sequence, reserve_a, reserve_b, total_shares,
               tvl, volume, fee_revenue, created_at
          FROM merge_source.liquidity_pool_snapshots
         WHERE ledger_sequence BETWEEN {lo} AND {hi}
        ON CONFLICT ON CONSTRAINT uq_lp_snapshots_pool_ledger DO NOTHING
        "#,
    )
    .await
}
