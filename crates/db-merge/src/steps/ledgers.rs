//! `ledgers` — natural PK on `sequence`, ranges disjoint by precondition.
//! Pure UNION. Mirror of `crates/indexer/.../write.rs::insert_ledger`
//! (~line 518).
//!
//! Ledger-windowed batching keeps WAL bounded — one snapshot is ~2M
//! rows; at 100k per batch that's ~20 SAVEPOINTs.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "ledgers",
        "merge_source.ledgers",
        "sequence",
        r#"
        INSERT INTO ledgers (sequence, hash, closed_at, protocol_version, transaction_count, base_fee)
        SELECT sequence, hash, closed_at, protocol_version, transaction_count, base_fee
          FROM merge_source.ledgers
         WHERE sequence BETWEEN {lo} AND {hi}
        ON CONFLICT (sequence) DO NOTHING
        "#,
    )
    .await
}
