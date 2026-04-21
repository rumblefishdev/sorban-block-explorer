//! Parse + persist a single ledger. Thin glue over existing crates —
//! all write-path logic lives in `indexer::handler::process::process_ledger`.

use aws_sdk_s3::Client as S3Client;
use indexer::handler::HandlerError;
use sqlx::PgPool;
use std::time::Instant;
use tracing::info;

use crate::error::BackfillError;
use crate::source;

/// Number of S3 fetch attempts per ledger before surfacing the error.
pub const FETCH_ATTEMPTS: u32 = 3;

/// Fetch, decompress, deserialize, and persist one ledger sequence.
pub async fn ingest_ledger(
    client: &S3Client,
    pool: &PgPool,
    seq: u32,
) -> Result<(), BackfillError> {
    let fetch_start = Instant::now();
    let compressed = source::fetch_ledger_with_retry(client, seq, FETCH_ATTEMPTS).await?;
    let fetch_ms = fetch_start.elapsed().as_millis();

    let parse_start = Instant::now();
    // Route pre-indexer parse errors through HandlerError so all parse/persist
    // failures land in a single BackfillError::Indexer variant — simpler retry
    // classifier downstream.
    let xdr_bytes = xdr_parser::decompress_zstd(&compressed).map_err(HandlerError::from)?;
    let batch = xdr_parser::deserialize_batch(&xdr_bytes).map_err(HandlerError::from)?;
    let parse_ms = parse_start.elapsed().as_millis();

    let persist_start = Instant::now();
    for meta in batch.ledger_close_metas.iter() {
        indexer::handler::process::process_ledger(meta, pool, None).await?;
    }
    let persist_ms = persist_start.elapsed().as_millis();

    info!(
        seq,
        bytes = compressed.len(),
        fetch_ms,
        parse_ms,
        persist_ms,
        "ledger ingested"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    //! End-to-end: public S3 fetch → parse → persist → re-run idempotency.
    //!
    //! Double-gated. Skips cleanly when either the DB or the public archive
    //! is unreachable, so `cargo test -p backfill-runner` stays green on
    //! workstations without network or Postgres. Exercised in anger via
    //! Step 8 (staging dry-run) in the task plan.
    //!
    //! Run locally:
    //!   DATABASE_URL=postgres://postgres:postgres@localhost:5432/soroban_block_explorer \
    //!       cargo test -p backfill-runner --lib -- --test-threads=1 --nocapture
    //!
    //! Assumes the staging DB has ADR 0027 migrations applied and partitions
    //! provisioned (or default partitions present) for the Soroban-era range.
    use super::*;
    use sqlx::PgPool;

    /// First Soroban-era ledger — stable, small, always present in the archive.
    const E2E_SEQ: u32 = 50_457_424;

    #[tokio::test]
    async fn ingest_real_ledger_is_idempotent() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL unset — skipping E2E ingest test");
            return;
        };
        let pool = match PgPool::connect(&url).await {
            Ok(p) => p,
            Err(err) => {
                eprintln!("DATABASE_URL unreachable ({err}) — skipping E2E ingest test");
                return;
            }
        };
        let client = crate::source::build_client().await;

        // Probe S3 first so a network-blocked workstation skips cleanly
        // instead of failing inside the ingest pipeline.
        match crate::source::fetch_ledger_with_retry(&client, E2E_SEQ, 1).await {
            Ok(_) => {}
            Err(err) => {
                eprintln!("public S3 unreachable ({err}) — skipping E2E ingest test");
                return;
            }
        }

        ingest_ledger(&client, &pool, E2E_SEQ)
            .await
            .expect("first ingest must succeed");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ledgers WHERE sequence = $1")
            .bind(i64::from(E2E_SEQ))
            .fetch_one(&pool)
            .await
            .expect("count query");
        assert_eq!(count, 1, "ledger row must exist after ingest");

        // Replay must be a no-op (existence check in process_ledger).
        ingest_ledger(&client, &pool, E2E_SEQ)
            .await
            .expect("replay ingest must be idempotent, not error");

        let count_after: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM ledgers WHERE sequence = $1")
                .bind(i64::from(E2E_SEQ))
                .fetch_one(&pool)
                .await
                .expect("count query");
        assert_eq!(count_after, 1, "replay must not duplicate the ledger row");
    }
}
