//! `nfts` — REMAP. FK `contract_id → soroban_contracts(id)` is remapped
//! via `merge_remap.soroban_contracts`. Mirror of indexer upsert at
//! `write.rs:1490`.
//!
//! `current_owner_id` (FK → accounts) and `current_owner_ledger` ARE
//! merged here with the indexer's watermark CASE clause (the higher
//! `current_owner_ledger` wins). Original task 0186 spec said "leave
//! NULL until finalize rebuilds from nft_ownership", but `nft_ownership`
//! turns out not to capture every transfer — the indexer also writes
//! `current_owner_*` directly via this upsert when parsing transfers.
//! Dropping the columns and rebuilding from a partial event log loses
//! data. Finalize remains as a fallback that only fires when an event
//! is fresher than the merged direct-upsert state.
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
                   n.metadata, n.minted_at_ledger,
                   ra.target_id AS current_owner_id_remapped,
                   n.current_owner_ledger
              FROM merge_source.nfts n
              JOIN merge_remap.soroban_contracts rsc ON rsc.source_id = n.contract_id
              LEFT JOIN merge_remap.accounts ra ON ra.source_id = n.current_owner_id
             WHERE n.id BETWEEN {lo} AND {hi}
        ),
        inserted AS (
            INSERT INTO nfts (
                contract_id, token_id, collection_name, name, media_url, metadata,
                minted_at_ledger, current_owner_id, current_owner_ledger
            )
            SELECT contract_id_remapped, token_id, collection_name, name, media_url,
                   metadata, minted_at_ledger,
                   current_owner_id_remapped, current_owner_ledger
              FROM input
            ON CONFLICT (contract_id, token_id) DO UPDATE SET
                collection_name  = COALESCE(EXCLUDED.collection_name, nfts.collection_name),
                name             = COALESCE(EXCLUDED.name, nfts.name),
                media_url        = COALESCE(EXCLUDED.media_url, nfts.media_url),
                metadata         = COALESCE(EXCLUDED.metadata, nfts.metadata),
                minted_at_ledger = COALESCE(nfts.minted_at_ledger, EXCLUDED.minted_at_ledger),
                current_owner_id = CASE
                    WHEN EXCLUDED.current_owner_ledger > COALESCE(nfts.current_owner_ledger, 0)
                    THEN EXCLUDED.current_owner_id
                    ELSE nfts.current_owner_id
                END,
                current_owner_ledger = GREATEST(
                    COALESCE(nfts.current_owner_ledger, 0),
                    COALESCE(EXCLUDED.current_owner_ledger, 0)
                )
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
