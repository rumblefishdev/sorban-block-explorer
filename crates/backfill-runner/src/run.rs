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

use indicatif::MultiProgress;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::bootstrap::bootstrap_account_state;
use crate::dashboard::{Dashboard, install_panic_hook};
use crate::error::BackfillError;
use crate::ingest::{PartitionStats, index_partition};
use crate::partition::{Partition, partitions_for_range};
use crate::sink::Sink;
use crate::sync::{AwsCliS3Driver, S3Driver, SyncOutcome, sync_partition};

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    sink: &Sink,
    temp_dir: &Path,
    start: u32,
    end: u32,
    keep_partitions: bool,
    soroban_rpc_url: Option<&str>,
    reindex: bool,
    mp: &MultiProgress,
) -> Result<(), BackfillError> {
    assert!(
        start <= end,
        "invalid range: start ({start}) must be <= end ({end})"
    );

    tokio::fs::create_dir_all(&temp_dir).await?;

    // Pre-flight. Either check failing means the run has no chance of
    // completing — panic loudly rather than produce a typed error.
    preflight_aws().await;
    preflight_sink(sink).await;

    let partitions = partitions_for_range(start, end);
    if partitions.is_empty() {
        info!("no partitions in range");
        return Ok(());
    }

    // `--reindex` bypasses the resume skip so an already-ingested range is
    // re-parsed to populate a NEW derived table over history (task 0359
    // `operation_asset_appearances`) — `load_completed` reads the `ledgers`
    // table, which the original ingest already filled, so without this every
    // partition reads as done. Empty completed-set → every ledger re-parsed;
    // all writes are ReplacingMergeTree-idempotent.
    // `--lp-amounts-only` implies the same bypass: its whole purpose is to
    // re-parse an ALREADY-ingested range for one new derived table, and it
    // writes no `ledgers` marker of its own, so the resume filter would skip
    // every ledger and the run would do nothing (task 0279).
    let completed = if reindex || sink.targeted() {
        std::collections::HashSet::new()
    } else {
        sink.load_completed(start, end).await?
    };

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
    // Task 0225: counter for partitions skipped due to S3 archive lag.
    // Operator visibility only — does not abort the run.
    let mut partitions_skipped_s3_incomplete: usize = 0;

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

    // Task 0225: S3 driver. Single concrete impl behind a trait so the
    // sync orchestrator is mockable. `Arc<dyn …>` lets us hand a clone
    // to the prefetch task without lifetime gymnastics.
    let driver: Arc<dyn S3Driver> = Arc::new(AwsCliS3Driver);

    // Prime: foreground sync of the first partition that still needs work.
    // Subsequent partitions arrive via the background prefetch spawned at
    // the end of each iteration. The sync outcome flows through the loop
    // — current iteration handles the outcome captured for *this*
    // partition; the prefetch handle resolves to next iteration's outcome.
    dashboard.set_partition(0, todo.len(), todo[0].start);
    dashboard.set_stage("syncing");
    let mut current_outcome = sync_partition(&*driver, todo[0], temp_dir).await?;

    for (i, partition) in todo.iter().enumerate() {
        dashboard.set_partition(i, todo.len(), partition.start);

        // Kick off prefetch for N+1 BEFORE handling N — so the sync runs
        // while the indexer is busy. Exactly one in flight.
        let next_handle: Option<JoinHandle<Result<SyncOutcome, BackfillError>>> =
            if let Some(next) = todo.get(i + 1) {
                let next = (*next).clone();
                let temp = temp_dir.to_path_buf();
                let driver_clone = Arc::clone(&driver);
                Some(tokio::spawn(async move {
                    sync_partition(&*driver_clone, &next, &temp).await
                }))
            } else {
                None
            };

        // Handle current partition's sync outcome.
        match current_outcome {
            SyncOutcome::Complete => {
                dashboard.set_stage("indexing");
                let stats = index_partition(
                    partition,
                    temp_dir,
                    sink,
                    start,
                    end,
                    &completed,
                    &dashboard,
                    keep_partitions,
                )
                .await?;

                // Fold per-partition stats into the run-wide accumulator.
                // `wall_clock` is per-partition and not summed.
                totals.indexed += stats.indexed;
                totals.skipped_completed += stats.skipped_completed;
                totals.total_bytes += stats.total_bytes;
                totals.parse_total_ms += stats.parse_total_ms;
                totals.persist_total_ms += stats.persist_total_ms;
                totals.min_ledger_ms = combine_min(totals.min_ledger_ms, stats.min_ledger_ms);
                totals.max_ledger_ms = combine_max(totals.max_ledger_ms, stats.max_ledger_ms);

                // Delete the local partition folder now that it has been
                // fully indexed. See `--keep-partitions` for the override.
                cleanup_partition_folder(partition, temp_dir, keep_partitions, &dashboard).await?;
            }
            SyncOutcome::S3Incomplete { local, s3, need } => {
                // The actual warn log already fired inside `sync_partition`
                // with the same context. Here we just account for the skip
                // and clean up any partial dir so a future rerun starts
                // from scratch.
                warn!(
                    partition_start = partition.start,
                    local, s3, need, "skipping partition — S3 archive lag"
                );
                partitions_skipped_s3_incomplete += 1;
                cleanup_partition_folder(partition, temp_dir, keep_partitions, &dashboard).await?;
            }
        }

        // Await prefetch so its error (if any) surfaces synchronously
        // before we advance. Happy path: already resolved, zero wait.
        // The returned `SyncOutcome` is carried into the next iteration's
        // current_outcome.
        current_outcome = if let Some(h) = next_handle {
            dashboard.set_stage("syncing");
            h.await.expect("prefetch task panicked")?
        } else {
            // Last iteration — no further work; value is unused after the
            // loop exits.
            SyncOutcome::Complete
        };
    }

    dashboard.finish_and_clear();

    // ---- Bootstrap account state (task 0214, audit §E06) ----------------
    //
    // The task body proposed running this **before** the per-ledger
    // ingest loop, but the discovery query reads CH's
    // `transaction_participants` — which on a fresh database is empty
    // until ingest populates it. Running the bootstrap **after** the
    // loop instead lets us scan the just-populated participants table,
    // identify skeleton accounts that the parser-emitted state never
    // touched in this window, and top up via Soroban RPC's
    // `getLedgerEntries`. Phase 2's incremental top-up gate
    // (sequence_number = 0 filter) is intrinsic to the discovery
    // query, so a re-run of the same window only fixes the rows that
    // still need it. Without `--soroban-rpc-url` the step
    // short-circuits with a single info log.
    match bootstrap_account_state(sink, soroban_rpc_url, start, end).await {
        Ok(stats) if stats.discovered > 0 || stats.staged_accounts > 0 => {
            info!(
                discovered = stats.discovered,
                fetched = stats.fetched,
                staged = stats.staged_accounts,
                batches = stats.rpc_batches,
                rpc_errors = stats.rpc_errors,
                "bootstrap account-state snapshot complete"
            );
        }
        Ok(_) => {
            // discovered=0 either means no RPC endpoint or no
            // skeletons to top up — already logged inside
            // `bootstrap_account_state`.
        }
        Err(err) => {
            // Don't fail the whole run on bootstrap failure — the
            // per-ledger ingest already succeeded and the rows are in
            // CH; bootstrap is opportunistic enrichment that the
            // operator can re-run by invoking the same range.
            tracing::warn!(%err, "bootstrap_account_state failed; continuing");
        }
    }

    let elapsed = run_start.elapsed();
    print_run_summary(
        todo.len(),
        &totals,
        partitions_skipped_s3_incomplete,
        elapsed,
    );

    Ok(())
}

/// Common cleanup: delete the partition's local folder unless
/// `--keep-partitions` is in effect, logging the choice either way.
/// Used on both the happy path (post-index) and the skip path
/// (S3 archive lag) so a future rerun starts from a clean state.
async fn cleanup_partition_folder(
    partition: &Partition,
    temp_dir: &Path,
    keep_partitions: bool,
    dashboard: &Dashboard,
) -> Result<(), BackfillError> {
    let local = partition.local_folder(temp_dir);
    if keep_partitions {
        info!(
            partition = partition.start,
            local = %local.display(),
            "partition local folder kept (--keep-partitions)"
        );
        return Ok(());
    }
    dashboard.set_stage("cleaning");
    match tokio::fs::remove_dir_all(&local).await {
        Ok(()) => {
            info!(
                partition = partition.start,
                local = %local.display(),
                "partition local folder cleaned up"
            );
            Ok(())
        }
        // Missing dir is fine — sync may have written nothing
        // (S3Incomplete skip path) or a previous attempt cleaned up.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(BackfillError::Io(e)),
    }
}

/// Final run summary — printed via `println!` so it's always visible,
/// not gated by `--verbose`. The per-ledger and per-partition info logs
/// are the debugging stream; this is the single "what just happened"
/// block an operator sees when a run wraps up.
fn print_run_summary(
    partitions_processed: usize,
    totals: &PartitionStats,
    partitions_skipped_s3_incomplete: usize,
    elapsed: Duration,
) {
    let (min_str, max_str) = match (totals.min_ledger_ms, totals.max_ledger_ms) {
        (Some(min), Some(max)) => (format!("{min} ms"), format!("{max} ms")),
        _ => ("n/a".into(), "n/a".into()),
    };
    println!();
    println!("=== backfill complete ===");
    println!("partitions processed:      {partitions_processed}");
    println!("partitions skipped (S3):   {partitions_skipped_s3_incomplete}");
    println!("ledgers indexed:           {}", totals.indexed);
    println!("ledgers already in DB:     {}", totals.skipped_completed);
    println!("total bytes:               {}", totals.total_bytes);
    println!("parse total:               {} ms", totals.parse_total_ms);
    println!("persist total:             {} ms", totals.persist_total_ms);
    println!("ledger time (min / max):   {min_str} / {max_str}");
    println!("elapsed:                   {} s", elapsed.as_secs());
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

async fn preflight_sink(sink: &Sink) {
    sink.preflight()
        .await
        .unwrap_or_else(|err| panic!("pre-flight: sink unreachable: {err}"));
    info!("pre-flight: sink reachable");
}
