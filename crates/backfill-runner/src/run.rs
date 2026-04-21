//! `run` subcommand — orchestrates the end-to-end backfill.
//!
//! Sequential implementation. Fetches from S3, parses, and persists ledgers
//! one-by-one.
//!
//! Resume: sequences already in `ledgers` are filtered out up-front, so
//! completed ledgers are never fetched from S3.

use std::time::{Duration, Instant};

use tracing::{info, warn};

use crate::error::BackfillError;
use crate::{ingest, resume, source};

/// Progress summary cadence (log every N ledgers handled).
const PROGRESS_EVERY: usize = 100;

// ---------------------------------------------------------------------------
// Public entry point (Layer 0)
// ---------------------------------------------------------------------------

pub async fn execute(
    database_url: &str,
    start: u32,
    end: u32,
    chunk_size: u32,
) -> Result<(), BackfillError> {
    if start > end {
        return Err(BackfillError::InvalidRange(format!(
            "start ({}) must be <= end ({})",
            start, end
        )));
    }
    if chunk_size < 1 {
        return Err(BackfillError::InvalidArgument(
            "chunk-size must be >= 1".to_string(),
        ));
    }

    let total_requested = (end - start + 1) as usize;
    let pool = db::pool::create_pool(database_url)?;
    let client = source::build_client().await;

    let completed = resume::load_completed(&pool, start, end).await?;
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
    let mut processed = 0usize;
    let mut gaps = 0usize;

    for seq in pending {
        match ingest::ingest_ledger(&client, &pool, seq).await {
            Ok(()) => {}
            Err(BackfillError::S3NotFound { key }) => {
                warn!(seq, %key, "ledger missing in public archive, skipping");
                missing.push(seq);
                gaps += 1;
            }
            Err(e) => return Err(e),
        }

        processed += 1;
        if processed % PROGRESS_EVERY == 0 || processed == pending_total {
            log_progress(processed, gaps, pending_total, run_start.elapsed());
        }
    }

    let elapsed = run_start.elapsed();
    let ingested = processed.saturating_sub(gaps);

    if missing.is_empty() {
        info!(
            ingested,
            elapsed_secs = elapsed.as_secs(),
            "backfill complete"
        );
    } else {
        warn!(
            ingested,
            gaps = missing.len(),
            elapsed_secs = elapsed.as_secs(),
            missing = ?missing,
            "backfill complete with archive gaps (not an error)"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Progress (Layer 1)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProgressSnapshot {
    pub processed: usize,
    pub ingested: usize,
    pub gaps: usize,
    pub total: usize,
    pub pct: f64,
    pub throughput: f64,
    pub eta_secs: u64,
}

pub(crate) fn progress_snapshot(
    processed: usize,
    gaps: usize,
    total: usize,
    elapsed: Duration,
) -> ProgressSnapshot {
    let elapsed_s = elapsed.as_secs_f64().max(0.001);
    let throughput = processed as f64 / elapsed_s;
    let remaining = total.saturating_sub(processed);
    let eta_secs = if throughput > 0.0 {
        (remaining as f64 / throughput) as u64
    } else {
        0
    };
    let pct = if total == 0 {
        0.0
    } else {
        (processed as f64 / total as f64) * 100.0
    };
    let ingested = processed.saturating_sub(gaps);
    ProgressSnapshot {
        processed,
        ingested,
        gaps,
        total,
        pct,
        throughput,
        eta_secs,
    }
}

fn log_progress(processed: usize, gaps: usize, total: usize, elapsed: Duration) {
    let s = progress_snapshot(processed, gaps, total, elapsed);
    info!(
        ingested = s.ingested,
        gaps = s.gaps,
        processed = s.processed,
        total = s.total,
        pct = format!("{:.1}%", s.pct),
        throughput = format!("{:.2} ledgers/s", s.throughput),
        eta_secs = s.eta_secs,
        "progress"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halfway_no_gaps() {
        let s = progress_snapshot(50, 0, 100, Duration::from_secs(50));
        assert_eq!(s.processed, 50);
        assert_eq!(s.ingested, 50);
        assert_eq!(s.gaps, 0);
        assert_eq!(s.total, 100);
        assert!((s.pct - 50.0).abs() < 0.001);
        assert!((s.throughput - 1.0).abs() < 0.001);
        assert_eq!(s.eta_secs, 50);
    }

    #[test]
    fn halfway_with_gaps() {
        let s = progress_snapshot(50, 10, 100, Duration::from_secs(50));
        assert_eq!(s.processed, 50);
        assert_eq!(s.ingested, 40);
        assert_eq!(s.gaps, 10);
        assert!((s.pct - 50.0).abs() < 0.001);
    }

    #[test]
    fn completed_has_zero_eta() {
        let s = progress_snapshot(100, 0, 100, Duration::from_secs(10));
        assert!((s.pct - 100.0).abs() < 0.001);
        assert_eq!(s.eta_secs, 0);
    }

    #[test]
    fn zero_elapsed_does_not_panic() {
        let s = progress_snapshot(10, 0, 100, Duration::from_secs(0));
        assert!(s.throughput.is_finite());
        assert!(s.throughput > 0.0);
    }

    #[test]
    fn zero_total_does_not_divide_by_zero() {
        let s = progress_snapshot(0, 0, 0, Duration::from_secs(1));
        assert_eq!(s.pct, 0.0);
        assert_eq!(s.eta_secs, 0);
    }
}
