//! Step 13 — rebuild `nfts.current_owner_id` / `current_owner_ledger`
//! from the merged `nft_ownership` event log per task 0186 + ADR 0040.
//!
//! `DISTINCT ON (nft_id) … ORDER BY nft_id, ledger_sequence DESC,
//! event_order DESC` picks the latest event per NFT — `event_type` is
//! mint/transfer/burn (ADR 0031); the row's `owner_id` is the resulting
//! owner (NULL for burn). Mirror semantics: the LWW indexer rule, but
//! reconstructed from full history rather than trusting either snapshot's
//! cached cache.
//!
//! Idempotent — `IS DISTINCT FROM` predicate skips rows already at the
//! correct value, so re-running yields zero affected rows.

use sqlx::PgConnection;

use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<u64, MergeError> {
    tracing::info!("finalize: rebuilding nfts.current_owner_* from nft_ownership");
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
           AND (n.current_owner_id     IS DISTINCT FROM latest.owner_id
             OR n.current_owner_ledger IS DISTINCT FROM latest.ledger_sequence)
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
