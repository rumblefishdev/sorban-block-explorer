//! `soroban_events_appearances` (partitioned) — FK rewrite via JOIN
//! `merge_remap.{soroban_contracts,transactions}`. PK is natural
//! `(contract_id, transaction_id, ledger_sequence, created_at)` →
//! `ON CONFLICT DO NOTHING`. Mirror of `write.rs:902`.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "soroban_events_appearances",
        "merge_source.soroban_events_appearances",
        "ledger_sequence",
        r#"
        INSERT INTO soroban_events_appearances (
            contract_id, transaction_id, ledger_sequence, amount, created_at
        )
        SELECT rsc.target_id, rt.target_id, sea.ledger_sequence, sea.amount, sea.created_at
          FROM merge_source.soroban_events_appearances sea
          JOIN merge_remap.soroban_contracts rsc ON rsc.source_id = sea.contract_id
          JOIN merge_remap.transactions rt
            ON rt.source_id = sea.transaction_id
           AND rt.created_at = sea.created_at
         WHERE sea.ledger_sequence BETWEEN {lo} AND {hi}
        ON CONFLICT (contract_id, transaction_id, ledger_sequence, created_at) DO NOTHING
        "#,
    )
    .await
}
