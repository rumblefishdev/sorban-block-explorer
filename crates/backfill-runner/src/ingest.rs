//! Parse + persist a single ledger. Thin glue over existing crates —
//! all write-path logic lives in `indexer::handler::process::process_ledger`.

use aws_sdk_s3::Client as S3Client;
use indexer::handler::HandlerError;
use sqlx::PgPool;
use std::time::Instant;
use tracing::info;

use crate::error::BackfillError;
use crate::source;

/// Fetch, decompress, deserialize, and persist one ledger sequence.
pub async fn ingest_ledger(
    client: &S3Client,
    pool: &PgPool,
    seq: u32,
) -> Result<(), BackfillError> {
    let fetch_start = Instant::now();
    let compressed = source::fetch_ledger(client, seq).await?;
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
