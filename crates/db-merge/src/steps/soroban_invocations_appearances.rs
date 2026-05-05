//! `soroban_invocations_appearances` (partitioned) — FK rewrite via JOIN
//! `merge_remap.{soroban_contracts,accounts,transactions}`. Mirror of
//! `write.rs:1060`.
//!
//! Two contract FKs:
//! - `contract_id` — NOT NULL, the trio's contract
//! - `caller_contract_id` — NULLABLE, the contract caller (DeFi router →
//!   pool sub-call); added by migration `20260430000000_invocations_caller_contract`
//!
//! `caller_id` is nullable too (G/M account caller). `ck_sia_caller_xor`
//! enforces at-most-one-non-null between `caller_id` and `caller_contract_id` —
//! source rows already satisfy it; we just preserve.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "soroban_invocations_appearances",
        "merge_source.soroban_invocations_appearances",
        "ledger_sequence",
        r#"
        INSERT INTO soroban_invocations_appearances (
            contract_id, transaction_id, ledger_sequence, caller_id,
            caller_contract_id, amount, created_at
        )
        SELECT rsc.target_id, rt.target_id, sia.ledger_sequence,
               rcaller.target_id, rcaller_c.target_id,
               sia.amount, sia.created_at
          FROM merge_source.soroban_invocations_appearances sia
          JOIN merge_remap.soroban_contracts rsc ON rsc.source_id = sia.contract_id
          JOIN merge_remap.transactions rt
            ON rt.source_id = sia.transaction_id
           AND rt.created_at = sia.created_at
          LEFT JOIN merge_remap.accounts rcaller             ON rcaller.source_id   = sia.caller_id
          LEFT JOIN merge_remap.soroban_contracts rcaller_c  ON rcaller_c.source_id = sia.caller_contract_id
         WHERE sia.ledger_sequence BETWEEN {lo} AND {hi}
        ON CONFLICT (contract_id, transaction_id, ledger_sequence, created_at) DO NOTHING
        "#,
    )
    .await
}
