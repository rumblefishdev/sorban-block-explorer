//! `run` subcommand — orchestrates the end-to-end backfill.
//!
//! Shape: one partition at a time, sequential per-ledger index, with a
//! **single-slot** background prefetch of partition *N+1* while *N* is
//! being indexed. No worker pool, no tokio `JoinSet` of indexer tasks —
//! concurrency is out of scope here (see task 0145).
//!
//! Pre-flight (`aws --version`, `SELECT 1`) **panics** on failure.
//! These are operator / environment errors, not transient conditions,
//! and the typed `BackfillError` is reserved for things worth catching
//! higher up.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use indexer::handler::persist::ClassificationCache;
use indicatif::MultiProgress;
use sqlx::PgPool;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tracing::info;

use crate::dashboard::{Dashboard, install_panic_hook};
use crate::error::BackfillError;
use crate::ingest::{PartitionStats, index_partition};
use crate::partition::{Partition, partitions_for_range};
use crate::resume::load_completed;
use crate::sync::sync_partition;

pub async fn execute(
    database_url: &str,
    temp_dir: &Path,
    start: u32,
    end: u32,
    mp: &MultiProgress,
) -> Result<(), BackfillError> {
    assert!(
        start <= end,
        "invalid range: start ({start}) must be <= end ({end})"
    );

    tokio::fs::create_dir_all(&temp_dir).await?;

    let pool = db::pool::create_pool(database_url)?;

    // Pre-flight. Either check failing means the run has no chance of
    // completing — panic loudly rather than produce a typed error.
    preflight_aws().await;
    preflight_db(&pool).await;

    let partitions = partitions_for_range(start, end);
    if partitions.is_empty() {
        info!("no partitions in range");
        return Ok(());
    }

    let completed = load_completed(&pool, start, end).await?;

    // Filter out partitions whose entire clamped range is already in the
    // `ledgers` table. With cleanup-after-index the local folder is gone
    // by the time the row lands in the DB, so a re-run without this
    // pre-sync filter would re-download ~1–2 GB per already-done
    // partition just for Stage B to reject every single persist call.
    // Per-ledger Stage B still matters for mid-partition crashes where
    // the partition is only partially in DB.
    let todo: Vec<&Partition> = partitions
        .iter()
        .filter(|p| !partition_fully_done(p, start, end, &completed))
        .collect();

    info!(
        start,
        end,
        partitions = partitions.len(),
        already_done_partitions = partitions.len() - todo.len(),
        to_process = todo.len(),
        already_ingested = completed.len(),
        "backfill starting"
    );

    if todo.is_empty() {
        info!("nothing to do — all partitions in range already fully indexed");
        return Ok(());
    }

    let run_start = Instant::now();
    let mut totals = PartitionStats::default();

    // Single cache instance reused across the whole run. Mirrors the
    // indexer Lambda's per-invocation reuse pattern (task 0118 Phase 2)
    // and the backfill-bench wiring — one batch `SELECT` per ledger for
    // unseen contracts, zero lookups for already-classified ones.
    let classification_cache = ClassificationCache::new();

    // Sticky dashboard. Visual bar covers the full range and is pre-
    // bumped by `completed.len()` (handled inside `Dashboard::new`);
    // `timing` is scoped only to the work this run actually has to do.
    // Widen to u64 before the arithmetic — `(end - start + 1) as u32`
    // wraps when `end == u32::MAX && start == 0`. Soroban ranges don't
    // hit that in practice, but the fix is free. Copilot review on PR #111.
    let total_range = u64::from(end) - u64::from(start) + 1;
    let already_done = completed.len() as u64;
    let dashboard = Arc::new(Dashboard::new(total_range, already_done, mp));

    install_panic_hook(dashboard.clone());

    // Prime: foreground sync of the first partition that still needs work.
    // Subsequent partitions arrive via the background prefetch spawned at
    // the end of each iteration.
    dashboard.set_partition(0, todo.len(), todo[0].start);
    dashboard.set_stage("syncing");
    sync_partition(todo[0], temp_dir).await?;

    for (i, partition) in todo.iter().enumerate() {
        dashboard.set_partition(i, todo.len(), partition.start);

        // Kick off prefetch for N+1 BEFORE indexing N — so the sync runs
        // while the indexer is busy. Exactly one in flight.
        let next_handle: Option<JoinHandle<Result<(), BackfillError>>> =
            if let Some(next) = todo.get(i + 1) {
                let next = (*next).clone();
                let temp = temp_dir.to_path_buf();
                Some(tokio::spawn(
                    async move { sync_partition(&next, &temp).await },
                ))
            } else {
                None
            };

        dashboard.set_stage("indexing");
        let stats = index_partition(
            partition,
            temp_dir,
            &pool,
            start,
            end,
            &completed,
            &dashboard,
            &classification_cache,
        )
        .await?;

        // Fold per-partition stats into the run-wide accumulator.
        // `wall_clock` is per-partition and not summed.
        totals.indexed += stats.indexed;
        totals.skipped_completed += stats.skipped_completed;
        totals.parse_total_ms += stats.parse_total_ms;
        totals.persist_total_ms += stats.persist_total_ms;
        totals.min_ledger_ms = combine_min(totals.min_ledger_ms, stats.min_ledger_ms);
        totals.max_ledger_ms = combine_max(totals.max_ledger_ms, stats.max_ledger_ms);

        // Delete the local partition folder now that it has been fully
        // indexed. Bounds total disk use at ~2 × partition_size (this
        // plus the prefetch in flight). Doing it BEFORE awaiting the
        // prefetch reclaims disk sooner in the sync-slower-than-index
        // case. A failure here is a hard error — silent warn-and-
        // continue would accumulate the garbage we just removed.
        dashboard.set_stage("cleaning");
        let local = partition.local_folder(temp_dir);
        tokio::fs::remove_dir_all(&local).await?;
        info!(
            partition = partition.start,
            local = %local.display(),
            "partition local folder cleaned up"
        );

        // Await prefetch so its error (if any) surfaces synchronously
        // before we advance. Happy path: already resolved, zero wait.
        // The returned `Duration` is dropped — per-partition sync time
        // lives in the `partition sync complete` tracing event.
        if let Some(h) = next_handle {
            dashboard.set_stage("syncing");
            h.await.expect("prefetch task panicked")?;
        }
    }

    dashboard.finish_and_clear();

    let elapsed = run_start.elapsed();
    print_run_summary(todo.len(), &totals, elapsed);

    Ok(())
}

/// Final run summary — printed via `println!` so it's always visible,
/// not gated by `--verbose`. The per-ledger and per-partition info logs
/// are the debugging stream; this is the single "what just happened"
/// block an operator sees when a run wraps up.
fn print_run_summary(partitions_processed: usize, totals: &PartitionStats, elapsed: Duration) {
    let (min_str, max_str) = match (totals.min_ledger_ms, totals.max_ledger_ms) {
        (Some(min), Some(max)) => (format!("{min} ms"), format!("{max} ms")),
        _ => ("n/a".into(), "n/a".into()),
    };
    println!();
    println!("=== backfill complete ===");
    println!("partitions processed:    {partitions_processed}");
    println!("ledgers indexed:         {}", totals.indexed);
    println!("ledgers already in DB:   {}", totals.skipped_completed);
    println!("parse total:             {} ms", totals.parse_total_ms);
    println!("persist total:           {} ms", totals.persist_total_ms);
    println!("ledger time (min / max): {min_str} / {max_str}");
    println!("elapsed:                 {} s", elapsed.as_secs());
}

fn combine_min(a: Option<u128>, b: Option<u128>) -> Option<u128> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn combine_max(a: Option<u128>, b: Option<u128>) -> Option<u128> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

/// Is every ledger in this partition's clamped range already in the DB?
///
/// "Clamped" = intersect the partition's full [start, end] with the run's
/// requested [start, end]. A partition at the edge of the range may only
/// need a subset of its ledgers, and that subset being complete is
/// sufficient to skip it entirely — sync + index.
fn partition_fully_done(
    partition: &Partition,
    start: u32,
    end: u32,
    completed: &HashSet<u32>,
) -> bool {
    let (first, last) = partition.clamped(start, end);
    (first..=last).all(|s| completed.contains(&s))
}

async fn preflight_aws() {
    let out = Command::new("aws")
        .arg("--version")
        .output()
        .await
        .unwrap_or_else(|err| {
            panic!(
                "pre-flight: failed to spawn `aws --version`: {err}. \
                 Is the AWS CLI installed and on PATH?"
            );
        });
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        panic!(
            "pre-flight: `aws --version` exited non-zero ({:?}): {}",
            out.status.code(),
            stderr
        );
    }
    info!(
        version = %String::from_utf8_lossy(&out.stdout).trim(),
        "pre-flight: aws CLI present"
    );
}

async fn preflight_db(pool: &PgPool) {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("pre-flight: database unreachable: {err}"));
    info!("pre-flight: database reachable");
}
