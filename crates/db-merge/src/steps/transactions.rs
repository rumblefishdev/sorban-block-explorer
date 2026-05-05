//! `transactions` (partitioned) — REMAP. FK `source_id → accounts(id)`
//! is remapped via `merge_remap.accounts`. Mirror of indexer upsert at
//! `write.rs:595` — `ON CONFLICT ON CONSTRAINT uq_transactions_hash_created_at
//! DO UPDATE SET hash = EXCLUDED.hash` (no-op UPDATE that still fires
//! RETURNING so we capture the target id on both insert and replay paths).
//!
//! Remap captures `(source_id, created_at, target_id)` because partition
//! routing on appearance-table FKs needs the composite `(transaction_id,
//! created_at)` — D3 JOINs on both columns.
//!
//! Batched by `ledger_sequence` — natural for transactions and aligns
//! with `backfill-runner` batch boundaries.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "transactions",
        "merge_source.transactions",
        "ledger_sequence",
        r#"
        WITH input AS (
            SELECT t.id AS src_id, t.hash, t.ledger_sequence, t.application_order,
                   ra.target_id AS source_id_remapped,
                   t.fee_charged, t.inner_tx_hash, t.successful, t.operation_count,
                   t.has_soroban, t.parse_error, t.created_at
              FROM merge_source.transactions t
              JOIN merge_remap.accounts ra ON ra.source_id = t.source_id
             WHERE t.ledger_sequence BETWEEN {lo} AND {hi}
        ),
        inserted AS (
            INSERT INTO transactions (
                hash, ledger_sequence, application_order, source_id, fee_charged,
                inner_tx_hash, successful, operation_count, has_soroban, parse_error, created_at
            )
            SELECT hash, ledger_sequence, application_order, source_id_remapped,
                   fee_charged, inner_tx_hash, successful, operation_count,
                   has_soroban, parse_error, created_at
              FROM input
            ON CONFLICT ON CONSTRAINT uq_transactions_hash_created_at
            DO UPDATE SET hash = EXCLUDED.hash
            RETURNING id, hash, created_at
        )
        INSERT INTO merge_remap.transactions (source_id, created_at, target_id)
        SELECT i.src_id, i.created_at, ins.id
          FROM input i
          JOIN inserted ins ON ins.hash = i.hash AND ins.created_at = i.created_at
        ON CONFLICT (source_id, created_at) DO UPDATE SET target_id = EXCLUDED.target_id
        "#,
    )
    .await
}
