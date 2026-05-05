//! `nfts` — REMAP. FK `contract_id → soroban_contracts(id)` is remapped
//! via `merge_remap.soroban_contracts`. Mirror of indexer upsert at
//! `write.rs:1490`.
//!
//! **`current_owner_id` and `current_owner_ledger` are intentionally
//! omitted** from both the INSERT column list and the DO UPDATE clause
//! per task 0186 §Step 3 #7 + ADR 0040. They are rebuilt from
//! `nft_ownership` in the `finalize` subcommand (Step 13). Trusting
//! either snapshot's cached values would corrupt LWW under cross-snapshot
//! load order.
//!
//! Batched by source `id` (SERIAL — INTEGER, not BIGINT). Batcher's
//! MIN/MAX query casts to BIGINT explicitly.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "nfts",
        "merge_source.nfts",
        "id",
        r#"
        WITH input AS (
            SELECT n.id AS src_id, rsc.target_id AS contract_id_remapped,
                   n.token_id, n.collection_name, n.name, n.media_url,
                   n.metadata, n.minted_at_ledger
              FROM merge_source.nfts n
              JOIN merge_remap.soroban_contracts rsc ON rsc.source_id = n.contract_id
             WHERE n.id BETWEEN {lo} AND {hi}
        ),
        inserted AS (
            INSERT INTO nfts (
                contract_id, token_id, collection_name, name, media_url, metadata, minted_at_ledger
            )
            SELECT contract_id_remapped, token_id, collection_name, name, media_url, metadata, minted_at_ledger
              FROM input
            ON CONFLICT (contract_id, token_id) DO UPDATE SET
                collection_name  = COALESCE(EXCLUDED.collection_name, nfts.collection_name),
                name             = COALESCE(EXCLUDED.name, nfts.name),
                media_url        = COALESCE(EXCLUDED.media_url, nfts.media_url),
                metadata         = COALESCE(EXCLUDED.metadata, nfts.metadata),
                minted_at_ledger = COALESCE(nfts.minted_at_ledger, EXCLUDED.minted_at_ledger)
            RETURNING id, contract_id, token_id
        )
        INSERT INTO merge_remap.nfts (source_id, target_id)
        SELECT i.src_id, ins.id
          FROM input i
          JOIN inserted ins ON ins.contract_id = i.contract_id_remapped
                           AND ins.token_id    = i.token_id
        ON CONFLICT (source_id) DO UPDATE SET target_id = EXCLUDED.target_id
        "#,
    )
    .await
}
