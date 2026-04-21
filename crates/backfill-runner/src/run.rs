//! `run` subcommand — orchestrates the end-to-end backfill.
//!
//! Phase B: serial single-ledger loop — no workers, no resume, no retry.
//! Phase C layers in: resume from DB, bounded mpsc + JoinSet workers,
//! retry/backoff, CancellationToken shutdown.

use tracing::{error, info};

use crate::error::BackfillError;
use crate::{ingest, source};

pub async fn execute(
    database_url: &str,
    start: u32,
    end: u32,
    _workers: usize,
    _chunk_size: u32,
) -> Result<(), BackfillError> {
    assert!(start <= end, "--start must be <= --end");

    let pool = db::pool::create_pool(database_url)?;
    let client = source::build_client().await;

    info!(start, end, total = end - start + 1, "backfill starting");

    let mut missing: Vec<u32> = Vec::new();
    for seq in start..=end {
        match ingest::ingest_ledger(&client, &pool, seq).await {
            Ok(()) => {}
            Err(BackfillError::S3NotFound { key }) => {
                tracing::warn!(seq, %key, "ledger missing in public archive, skipping");
                missing.push(seq);
            }
            Err(e) => {
                error!(seq, error = %e, "ingest failed");
                return Err(e);
            }
        }
    }

    if missing.is_empty() {
        info!("backfill complete");
        Ok(())
    } else {
        error!(count = missing.len(), missing = ?missing, "backfill finished with missing ledgers");
        Err(BackfillError::MissingLedgers(missing))
    }
}
