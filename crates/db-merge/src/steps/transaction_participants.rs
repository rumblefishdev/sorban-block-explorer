//! `transaction_participants` (partitioned) — FK rewrite via JOIN
//! `merge_remap.{transactions,accounts}`. Mirror of `write.rs:700`.
//! PK is natural `(account_id, created_at, transaction_id)` so dedup
//! falls out of `ON CONFLICT DO NOTHING`.
//!
//! Batched by source `transaction_id` — this table has no `ledger_sequence`
//! column, but source's BIGSERIAL transaction ids are dense, so source-id
//! windowing gives stable batches (~3-5 participants per transaction →
//! ~20-30k transactions per 100k-row batch).

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "transaction_participants",
        "merge_source.transaction_participants",
        "transaction_id",
        r#"
        INSERT INTO transaction_participants (transaction_id, account_id, created_at)
        SELECT rt.target_id, ra.target_id, tp.created_at
          FROM merge_source.transaction_participants tp
          JOIN merge_remap.transactions rt
            ON rt.source_id = tp.transaction_id
           AND rt.created_at = tp.created_at
          JOIN merge_remap.accounts ra ON ra.source_id = tp.account_id
         WHERE tp.transaction_id BETWEEN {lo} AND {hi}
        ON CONFLICT (account_id, created_at, transaction_id) DO NOTHING
        "#,
    )
    .await
}
