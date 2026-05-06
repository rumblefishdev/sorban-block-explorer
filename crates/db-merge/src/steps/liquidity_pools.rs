//! `liquidity_pools` — `pool_id` natural key, LEAST(`created_at_ledger`)
//! merge per ADR 0040. Mirror of `write.rs:1643-1645`.
//!
//! FK columns `asset_a/b_issuer_id → accounts(id)` are remapped via
//! `merge_remap.accounts`. LEFT JOIN handles the native pool case where
//! both issuer FKs are NULL.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, single};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    // Liquidity pools count is small (~thousands on full mainnet) so a
    // single batch is fine. If this grows we'll switch to
    // `created_at_ledger`-windowed batching.
    single(
        conn,
        "liquidity_pools",
        r#"
        INSERT INTO liquidity_pools (
            pool_id, asset_a_type, asset_a_code, asset_a_issuer_id,
            asset_b_type, asset_b_code, asset_b_issuer_id,
            fee_bps, created_at_ledger
        )
        SELECT lp.pool_id, lp.asset_a_type, lp.asset_a_code, ra_a.target_id,
               lp.asset_b_type, lp.asset_b_code, ra_b.target_id,
               lp.fee_bps, lp.created_at_ledger
          FROM merge_source.liquidity_pools lp
          LEFT JOIN merge_remap.accounts ra_a ON ra_a.source_id = lp.asset_a_issuer_id
          LEFT JOIN merge_remap.accounts ra_b ON ra_b.source_id = lp.asset_b_issuer_id
        ON CONFLICT (pool_id) DO UPDATE SET
            asset_a_type = liquidity_pools.asset_a_type,
            created_at_ledger = LEAST(liquidity_pools.created_at_ledger,
                                      EXCLUDED.created_at_ledger)
        "#,
    )
    .await
}
