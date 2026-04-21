//! `run` subcommand — orchestrates the end-to-end backfill.
//!
//! Current implementation is sequential (single-worker). The worker pool
//! (planner → bounded mpsc → `JoinSet`) is a follow-up; the `--workers`
//! and `--chunk-size` flags are accepted but not yet wired.
//!
//! Resume: sequences already in `ledgers` are filtered out up-front, so
//! completed ledgers are never fetched from S3.

use std::time::{Duration, Instant};

use tracing::{error, info, warn};

use crate::error::BackfillError;
use crate::{ingest, resume, source};

/// Progress summary cadence (log every N ledgers completed).
const PROGRESS_EVERY: usize = 100;

pub async fn execute(
    database_url: &str,
    start: u32,
    end: u32,
    workers: usize,
    chunk_size: u32,
) -> Result<(), BackfillError> {
    assert!(start <= end, "--start must be <= --end");
    assert!(workers >= 1, "--workers must be >= 1");
    assert!(chunk_size >= 1, "--chunk-size must be >= 1");

    let total_requested = (end - start + 1) as usize;
    let pool = db::pool::create_pool(database_url)?;
    let client = source::build_client().await;

    let completed: std::collections::HashSet<u32> =
        resume::load_completed(&pool, start, end).await?;
    let pending: Vec<u32> = (start..=end).filter(|s| !completed.contains(s)).collect();
    let pending_total = pending.len();

    info!(
        start,
        end,
        total = total_requested,
        already_ingested = completed.len(),
        pending = pending_total,
        "backfill starting"
    );

    if pending.is_empty() {
        info!("nothing to do — range already fully ingested");
        return Ok(());
    }

    let run_start = Instant::now();
    let mut missing: Vec<u32> = Vec::new();
    let mut done: usize = 0;

    for seq in pending {
        match ingest::ingest_ledger(&client, &pool, seq).await {
            Ok(()) => {}
            Err(BackfillError::S3NotFound { key }) => {
                warn!(seq, %key, "ledger missing in public archive, skipping");
                missing.push(seq);
            }
            Err(e) => {
                error!(seq, error = %e, "ingest failed");
                return Err(e);
            }
        }

        done += 1;
        if done.is_multiple_of(PROGRESS_EVERY) || done == pending_total {
            log_progress(done, pending_total, run_start.elapsed());
        }
    }

    let elapsed = run_start.elapsed();
    if missing.is_empty() {
        info!(done, elapsed_secs = elapsed.as_secs(), "backfill complete");
    } else {
        warn!(
            done,
            skipped = missing.len(),
            elapsed_secs = elapsed.as_secs(),
            missing = ?missing,
            "backfill complete with archive gaps (not an error)"
        );
    }
    Ok(())
}

fn log_progress(done: usize, total: usize, elapsed: Duration) {
    let elapsed_s = elapsed.as_secs_f64().max(0.001);
    let throughput = done as f64 / elapsed_s;
    let remaining = total.saturating_sub(done);
    let eta_secs = if throughput > 0.0 {
        (remaining as f64 / throughput) as u64
    } else {
        0
    };
    let pct = (done as f64 / total as f64) * 100.0;
    info!(
        done,
        total,
        pct = format!("{:.1}%", pct),
        throughput = format!("{:.2} ledgers/s", throughput),
        eta_secs,
        "progress"
    );
}
