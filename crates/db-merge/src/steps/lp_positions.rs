//! `lp_positions` — watermark UPSERT. Mirror of `write.rs:1749-1754`.
//! Crucial: naive `DO UPDATE SET shares = EXCLUDED.shares` would pick
//! whichever snapshot loaded last; we must compare `last_updated_ledger`
//! and only overwrite when the incoming row is strictly fresher.
//!
//! FK columns: `pool_id` is BYTEA natural (no remap needed —
//! `liquidity_pools` step already populated target's pools);
//! `account_id` → `accounts(id)` via `merge_remap.accounts`.
//!
//! Single batch — `lp_positions` row count is bounded (per-pool,
//! per-account). If T6 reveals scale issues we'll batch by `account_id`.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, single};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    single(
        conn,
        "lp_positions",
        r#"
        INSERT INTO lp_positions (
            pool_id, account_id, shares, first_deposit_ledger, last_updated_ledger
        )
        SELECT lpp.pool_id, ra.target_id, lpp.shares,
               lpp.first_deposit_ledger, lpp.last_updated_ledger
          FROM merge_source.lp_positions lpp
          JOIN merge_remap.accounts ra ON ra.source_id = lpp.account_id
        ON CONFLICT (pool_id, account_id) DO UPDATE SET
            shares = CASE
                WHEN EXCLUDED.last_updated_ledger >= lp_positions.last_updated_ledger
                THEN EXCLUDED.shares
                ELSE lp_positions.shares
            END,
            last_updated_ledger  = GREATEST(lp_positions.last_updated_ledger,
                                            EXCLUDED.last_updated_ledger),
            first_deposit_ledger = LEAST(lp_positions.first_deposit_ledger,
                                         EXCLUDED.first_deposit_ledger)
        "#,
    )
    .await
}
