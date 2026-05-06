//! Step 13 — rebuild `nfts.current_owner_id` / `current_owner_ledger`
//! from the merged `nft_ownership` event log per task 0186 + ADR 0040.
//!
//! `DISTINCT ON (nft_id) … ORDER BY nft_id, ledger_sequence DESC,
//! event_order DESC` picks the latest event per NFT — `event_type` is
//! mint/transfer/burn (ADR 0031); the row's `owner_id` is the resulting
//! owner (NULL for burn).
//!
//! **Fallback semantics, not authoritative.** `nft_ownership` does not
//! capture every transfer (the indexer also writes `current_owner_*`
//! directly via the nfts upsert when parsing transfers). The nfts merge
//! step (`steps/nfts.rs`) already maintains `current_owner_*` with the
//! indexer's watermark CASE; this finalize step only overwrites when an
//! event in `nft_ownership` is **strictly fresher** than the merged
//! direct-upsert state — otherwise we'd corrupt valid ownership with
//! stale event data (or NULL, when no events exist).
//!
//! Idempotent — `>` on `ledger_sequence` skips already-fresher rows.

use sqlx::PgConnection;

use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<u64, MergeError> {
    tracing::info!(
        "finalize: rebuilding nfts.current_owner_* from nft_ownership (only where event is fresher)"
    );
    let res = sqlx::query(
        r#"
        WITH latest AS (
            SELECT DISTINCT ON (nft_id)
                   nft_id, owner_id, ledger_sequence
              FROM nft_ownership
             ORDER BY nft_id, ledger_sequence DESC, event_order DESC
        )
        UPDATE nfts n
           SET current_owner_id     = latest.owner_id,
               current_owner_ledger = latest.ledger_sequence
          FROM latest
         WHERE n.id = latest.nft_id
           AND latest.ledger_sequence > COALESCE(n.current_owner_ledger, 0)
        "#,
    )
    .execute(&mut *conn)
    .await?;

    tracing::info!(
        rows = res.rows_affected(),
        "finalize: nfts.current_owner_* rebuilt"
    );
    Ok(res.rows_affected())
}
