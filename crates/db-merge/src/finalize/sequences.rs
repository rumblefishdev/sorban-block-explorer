//! Step 14 — `setval` all 7 surrogate-id sequences to `MAX(id)` per
//! ADR 0040 + task 0186. Without this, the live indexer's next
//! `nextval` would collide with merged rows.
//!
//! Per ADR 0040 §"Surrogate-id remap procedure", these are the seven
//! BIGSERIAL/SERIAL sequences exposed by the schema. List is hardcoded
//! here; if migrations add a new sequence, append it.
//!
//! Idempotent — running twice with no intervening writes is a no-op.
//! `setval` against an empty table is a no-op (we skip the call).

use sqlx::{PgConnection, Row};

use crate::error::MergeError;

const SEQUENCES: &[(&str, &str)] = &[
    ("accounts_id_seq", "accounts"),
    ("soroban_contracts_id_seq", "soroban_contracts"),
    ("assets_id_seq", "assets"),
    ("nfts_id_seq", "nfts"),
    ("transactions_id_seq", "transactions"),
    (
        "liquidity_pool_snapshots_id_seq",
        "liquidity_pool_snapshots",
    ),
    ("operations_appearances_id_seq", "operations_appearances"),
];

pub async fn run(conn: &mut PgConnection) -> Result<u32, MergeError> {
    let mut adjusted = 0u32;
    for (seq, table) in SEQUENCES {
        let row = sqlx::query(&format!("SELECT MAX(id)::bigint AS m FROM {table}"))
            .fetch_one(&mut *conn)
            .await?;
        let max: Option<i64> = row.try_get("m")?;
        match max {
            Some(m) => {
                sqlx::query("SELECT setval($1, $2)")
                    .bind(*seq)
                    .bind(m)
                    .execute(&mut *conn)
                    .await?;
                tracing::info!(sequence = seq, set_to = m, "finalize: setval");
                adjusted += 1;
            }
            None => {
                tracing::info!(
                    sequence = seq,
                    "finalize: empty table — leaving sequence at default"
                );
            }
        }
    }
    Ok(adjusted)
}
