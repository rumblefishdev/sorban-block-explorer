//! `assets` — dedup-only via partial UNIQUEs (no FK referrers, no remap
//! needed). Three INSERT statements mirroring the indexer's three asset
//! upsert paths in `write.rs:1077-1329`:
//!
//! 1. **Native** (`asset_type = 0`) — at most one row total, partial
//!    UNIQUE `uidx_assets_native`. NOT EXISTS guard.
//! 2. **Classic-like** (`asset_type IN (1, 2)` with `asset_code` +
//!    `issuer_id`) — partial UNIQUE `uidx_assets_classic_asset`.
//!    `asset_type = GREATEST(...)` for the monotonic ClassicCredit→Sac
//!    promotion (task 0160).
//! 3. **Contract-keyed** (`asset_type IN (2, 3)` with `contract_id`,
//!    NULL code+issuer) — partial UNIQUE `uidx_assets_soroban`.
//!    SAC-prefer guard from `write.rs:1311-1314`.
//!
//! FKs that need remapping in paths 2 + 3:
//! - `issuer_id → accounts(id)` via `merge_remap.accounts`
//! - `contract_id → soroban_contracts(id)` via `merge_remap.soroban_contracts`
//!
//! Three single-batch executions — assets count is bounded (~thousands
//! on full mainnet); ledger windowing buys nothing.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, single};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    let mut total = MergeStats::default();

    let native = single(
        conn,
        "assets:native",
        r#"
        INSERT INTO assets (asset_type, name, total_supply, holder_count)
        SELECT s.asset_type, s.name, s.total_supply, s.holder_count
          FROM merge_source.assets s
         WHERE s.asset_type = 0
           AND NOT EXISTS (SELECT 1 FROM assets WHERE asset_type = 0)
        "#,
    )
    .await?;
    total.batches += native.batches;
    total.rows_affected += native.rows_affected;

    let classic = single(
        conn,
        "assets:classic_like",
        r#"
        INSERT INTO assets (asset_type, asset_code, issuer_id, contract_id,
                            name, total_supply, holder_count)
        SELECT s.asset_type, s.asset_code, ra.target_id, rsc.target_id,
               s.name, s.total_supply, s.holder_count
          FROM merge_source.assets s
          JOIN merge_remap.accounts ra ON ra.source_id = s.issuer_id
          LEFT JOIN merge_remap.soroban_contracts rsc ON rsc.source_id = s.contract_id
         WHERE s.asset_type IN (1, 2)
           AND s.asset_code IS NOT NULL
           AND s.issuer_id  IS NOT NULL
        ON CONFLICT (asset_code, issuer_id) WHERE asset_type IN (1, 2) DO UPDATE SET
            asset_type   = GREATEST(EXCLUDED.asset_type, assets.asset_type),
            contract_id  = COALESCE(EXCLUDED.contract_id, assets.contract_id),
            name         = COALESCE(EXCLUDED.name, assets.name),
            total_supply = COALESCE(EXCLUDED.total_supply, assets.total_supply),
            holder_count = COALESCE(EXCLUDED.holder_count, assets.holder_count)
        "#,
    )
    .await?;
    total.batches += classic.batches;
    total.rows_affected += classic.rows_affected;

    let contract_keyed = single(
        conn,
        "assets:contract_keyed",
        r#"
        INSERT INTO assets (asset_type, contract_id, name, total_supply, holder_count)
        SELECT s.asset_type, rsc.target_id, s.name, s.total_supply, s.holder_count
          FROM merge_source.assets s
          JOIN merge_remap.soroban_contracts rsc ON rsc.source_id = s.contract_id
         WHERE s.asset_type IN (2, 3)
           AND s.asset_code IS NULL
           AND s.issuer_id  IS NULL
        ON CONFLICT (contract_id) WHERE asset_type IN (2, 3) DO UPDATE SET
            asset_type   = CASE
                WHEN assets.asset_type = 2 OR EXCLUDED.asset_type = 2 THEN 2
                ELSE EXCLUDED.asset_type
            END,
            name         = COALESCE(EXCLUDED.name, assets.name),
            total_supply = COALESCE(EXCLUDED.total_supply, assets.total_supply),
            holder_count = COALESCE(EXCLUDED.holder_count, assets.holder_count)
        "#,
    )
    .await?;
    total.batches += contract_keyed.batches;
    total.rows_affected += contract_keyed.rows_affected;

    Ok(total)
}
