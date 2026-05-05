//! `operations_appearances` (partitioned) — dedup-only via wide
//! UNIQUE `uq_ops_app_identity (NULLS NOT DISTINCT)`. Mirror of
//! `write.rs:806`. No FK referrers; `id BIGSERIAL` autoallocates fresh.
//!
//! FK columns rewritten through `merge_remap`:
//! - `transaction_id` + `created_at` → `merge_remap.transactions` (composite)
//! - `source_id`, `destination_id`, `asset_issuer_id` → `merge_remap.accounts` (nullable, LEFT JOIN)
//! - `contract_id` → `merge_remap.soroban_contracts` (nullable, LEFT JOIN)
//! - `pool_id` is BYTEA natural key, no remap

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "operations_appearances",
        "merge_source.operations_appearances",
        "ledger_sequence",
        r#"
        INSERT INTO operations_appearances (
            transaction_id, type, source_id, destination_id,
            contract_id, asset_code, asset_issuer_id, pool_id,
            amount, ledger_sequence, created_at
        )
        SELECT rt.target_id, oa.type,
               rs.target_id, rd.target_id, rsc.target_id,
               oa.asset_code, ri.target_id, oa.pool_id,
               oa.amount, oa.ledger_sequence, oa.created_at
          FROM merge_source.operations_appearances oa
          JOIN merge_remap.transactions rt
            ON rt.source_id = oa.transaction_id
           AND rt.created_at = oa.created_at
          LEFT JOIN merge_remap.accounts rs  ON rs.source_id  = oa.source_id
          LEFT JOIN merge_remap.accounts rd  ON rd.source_id  = oa.destination_id
          LEFT JOIN merge_remap.accounts ri  ON ri.source_id  = oa.asset_issuer_id
          LEFT JOIN merge_remap.soroban_contracts rsc ON rsc.source_id = oa.contract_id
         WHERE oa.ledger_sequence BETWEEN {lo} AND {hi}
        ON CONFLICT ON CONSTRAINT uq_ops_app_identity DO NOTHING
        "#,
    )
    .await
}
