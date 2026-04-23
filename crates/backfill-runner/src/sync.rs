//! `aws s3 sync` driver for one partition.
//!
//! Unit of work: one whole 64k-ledger partition, downloaded via the AWS CLI
//! into the local temp directory. The CLI subprocess is deliberate — `aws
//! s3 sync` is the right tool here (listing, parallel GETs, dedup, resume
//! on partial downloads), and reimplementing it against `aws-sdk-s3` is
//! not justified (see ADR context in task 0145).
//!
//! Stage A resume — there is no marker, no manifest, no file-count check.
//! `aws s3 sync` is **itself idempotent**: a second call against an already
//! complete local dir is a LIST + no GETs (seconds, not minutes). So we
//! just always run it. If the previous run crashed mid-sync, the partial
//! dir gets filled in by the next sync on its own. The real resume filter
//! for duplicate work lives in Stage B (the `ledgers` table).

use std::path::Path;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tracing::{info, warn};

use crate::error::BackfillError;
use crate::partition::Partition;

// Retry policy for the `aws s3 sync` subprocess (task 0145 decision).
// Hardcoded — not operator-tunable; change the constants if the numbers drift.
const RETRY_ATTEMPTS: u32 = 3;
const RETRY_BASE_DELAY: Duration = Duration::from_secs(2);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(30);
const RETRY_MULTIPLIER: u32 = 2;

/// Sync one partition from S3 to `temp_dir`. Idempotent by virtue of
/// `aws s3 sync` itself — a second call over a fully-synced dir is a
/// cheap LIST with no GETs. Sync duration is surfaced via the
/// `partition sync complete` tracing event; no return value carries it.
pub async fn sync_partition(partition: &Partition, temp_dir: &Path) -> Result<(), BackfillError> {
    let local = partition.local_folder(temp_dir);
    tokio::fs::create_dir_all(&local).await?;

    let duration = run_sync_with_retry(partition, &local).await?;
    let (file_count, total_bytes) = dir_stats(&local).await?;

    info!(
        partition = partition.start,
        sync_duration_ms = duration.as_millis(),
        file_count,
        total_bytes,
        "partition sync complete"
    );

    Ok(())
}

/// Run `aws s3 sync` with exponential backoff. Returns the duration of
/// the **successful** attempt — retries are operator-visible via `warn!`
/// events and don't contaminate the reported sync time.
async fn run_sync_with_retry(
    partition: &Partition,
    local: &Path,
) -> Result<Duration, BackfillError> {
    let mut delay = RETRY_BASE_DELAY;
    for attempt in 1..=RETRY_ATTEMPTS {
        let start = Instant::now();
        match run_sync_once(partition, local).await {
            Ok(()) => return Ok(start.elapsed()),
            Err(err) if attempt == RETRY_ATTEMPTS => return Err(err),
            Err(err) => {
                warn!(
                    partition = partition.start,
                    attempt,
                    error = %err,
                    retry_in_secs = delay.as_secs(),
                    "aws s3 sync failed, retrying"
                );
                tokio::time::sleep(delay).await;
                delay = (delay.saturating_mul(RETRY_MULTIPLIER)).min(RETRY_MAX_DELAY);
            }
        }
    }
    unreachable!("retry loop exits via return")
}

/// Spawn one `aws s3 sync` invocation. Returns `Ok(())` on exit 0,
/// `Err(AwsSyncFailed)` on any non-zero exit (caller layers retry).
async fn run_sync_once(partition: &Partition, local: &Path) -> Result<(), BackfillError> {
    let s3 = partition.s3_folder();

    info!(
        partition = partition.start,
        s3 = %s3,
        local = %local.display(),
        "running aws s3 sync"
    );

    let output = Command::new("aws")
        .arg("s3")
        .arg("sync")
        .arg(&s3)
        .arg(local)
        .arg("--no-sign-request")
        .arg("--quiet")
        .output()
        .await?;

    if output.status.success() {
        return Ok(());
    }

    // Trim stderr so the error message stays log-friendly. Full output is
    // already in the subprocess's own streams if `--quiet` is dropped.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let trimmed = stderr.chars().take(2_000).collect::<String>();

    Err(BackfillError::AwsSyncFailed {
        partition: partition.start,
        exit_code: output.status.code().unwrap_or(-1),
        stderr: trimmed,
    })
}

/// Count `.xdr.zst` files and sum their bytes in a synced partition dir.
/// Non-ledger files are ignored.
async fn dir_stats(dir: &Path) -> Result<(usize, u64), BackfillError> {
    let mut entries = tokio::fs::read_dir(dir).await?;
    let mut count = 0usize;
    let mut bytes = 0u64;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".xdr.zst") {
            warn!("skipping non-ledger file: {}", name);
            continue;
        }
        let meta = entry.metadata().await?;
        count += 1;
        bytes += meta.len();
    }
    Ok((count, bytes))
}

#[cfg(test)]
mod tests {
    //! Stage A behavior tests — no network, no subprocess.
    //!
    //! The retry loop and the subprocess wiring are exercised end-to-end in
    //! the staging dry-run (task 0145 plan, Step 8). Here we lock the retry
    //! constants against the spec so drift is a compile-less signal.
    use super::*;

    #[test]
    fn retry_constants_match_spec() {
        // Lock in the numbers called out in task 0145: 3 attempts, 2s base,
        // ×2, 30s cap. Drift here is a silent regression of the operator
        // contract.
        assert_eq!(RETRY_ATTEMPTS, 3);
        assert_eq!(RETRY_BASE_DELAY, Duration::from_secs(2));
        assert_eq!(RETRY_MAX_DELAY, Duration::from_secs(30));
        assert_eq!(RETRY_MULTIPLIER, 2);
    }
}
