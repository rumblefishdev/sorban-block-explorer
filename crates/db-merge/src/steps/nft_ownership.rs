//! `nft_ownership` (partitioned, full event log) — FK rewrite via JOIN
//! `merge_remap.{nfts,accounts,transactions}`. Mirror of `write.rs:1567`.
//! PK is natural `(nft_id, created_at, ledger_sequence, event_order)` →
//! `ON CONFLICT DO NOTHING` for replay safety.
//!
//! `owner_id` is nullable (burns leave it NULL) — LEFT JOIN.
//! Finalize (Step 13 in task 0186) reads from this table to rebuild
//! `nfts.current_owner_*`, so getting every event row right is critical.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "nft_ownership",
        "merge_source.nft_ownership",
        "ledger_sequence",
        r#"
        INSERT INTO nft_ownership (
            nft_id, transaction_id, owner_id, event_type,
            ledger_sequence, event_order, created_at
        )
        SELECT rn.target_id, rt.target_id, rowner.target_id, no.event_type,
               no.ledger_sequence, no.event_order, no.created_at
          FROM merge_source.nft_ownership no
          JOIN merge_remap.nfts rn ON rn.source_id = no.nft_id
          JOIN merge_remap.transactions rt
            ON rt.source_id = no.transaction_id
           AND rt.created_at = no.created_at
          LEFT JOIN merge_remap.accounts rowner ON rowner.source_id = no.owner_id
         WHERE no.ledger_sequence BETWEEN {lo} AND {hi}
        ON CONFLICT (nft_id, created_at, ledger_sequence, event_order) DO NOTHING
        "#,
    )
    .await
}
