//! Local backfill — partition-based download + pipelined indexing.
//!
//! Downloads whole S3 partitions (64k ledgers each) via `aws s3 sync`.
//! Pipeline: download partition N+1 in background while indexing partition N.
//! First partition is downloaded before indexing starts (cold start).
//!
//! Usage:
//!   cargo run -p backfill-bench -- --start 62016000 --end 62016099
//!   cargo run -p backfill-bench -- --start 62000000 --end 62300000

use chrono::Local;
use clap::Parser;
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{error, info, warn};

const S3_BUCKET_BASE: &str = "s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet";
const PARTITION_SIZE: u32 = 64000;
const TEMP_DIR: &str = ".temp";

#[derive(Parser)]
#[command(name = "backfill-bench", about = "Local backfill benchmark")]
struct Args {
    /// First ledger to index (inclusive)
    #[arg(long)]
    start: u32,

    /// Last ledger to index (inclusive)
    #[arg(long)]
    end: u32,

    /// PostgreSQL connection string (flag > DATABASE_URL env > default)
    #[arg(
        long,
        env = "DATABASE_URL",
        default_value = "postgres://postgres:postgres@127.0.0.1:5432/soroban_block_explorer"
    )]
    database_url: String,

    /// Delete partition files after indexing
    #[arg(long, default_value_t = false)]
    cleanup: bool,
}

// ---------------------------------------------------------------------------
// Partition helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Partition {
    /// First ledger in the partition (aligned to PARTITION_SIZE)
    start: u32,
    /// Last ledger in the partition
    end: u32,
    /// Hex prefix for the S3 folder name
    hex: String,
}

impl Partition {
    fn from_ledger(ledger: u32) -> Self {
        let start = ledger - (ledger % PARTITION_SIZE);
        let end = start + PARTITION_SIZE - 1;
        let hex = format!("{:08X}", u32::MAX - start);
        Self { start, end, hex }
    }

    fn s3_folder(&self) -> String {
        format!(
            "{S3_BUCKET_BASE}/{}--{}-{}/",
            self.hex, self.start, self.end
        )
    }

    fn local_folder(&self) -> PathBuf {
        Path::new(TEMP_DIR).join(format!("{}--{}-{}", self.hex, self.start, self.end))
    }
}

/// Compute the list of partitions covering [start, end].
fn partitions_for_range(start: u32, end: u32) -> Vec<Partition> {
    let mut result = Vec::new();
    let mut cursor = start;
    while cursor <= end {
        let p = Partition::from_ledger(cursor);
        result.push(p.clone());
        cursor = p.end + 1;
    }
    result
}

/// Local path for a ledger file within its partition folder.
fn local_ledger_path(partition: &Partition, ledger: u32) -> PathBuf {
    let file_hex = format!("{:08X}", u32::MAX - ledger);
    partition
        .local_folder()
        .join(format!("{file_hex}--{ledger}.xdr.zst"))
}

// ---------------------------------------------------------------------------
// Download (aws s3 sync)
// ---------------------------------------------------------------------------

/// Sync an entire partition from S3 to local disk.
/// Returns Ok(true) if sync succeeded, Ok(false) if skipped (already exists).
async fn sync_partition(
    partition: &Partition,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let local = partition.local_folder();
    let s3 = partition.s3_folder();

    // Skip if folder already has files (previous run)
    if local.exists() {
        let count = std::fs::read_dir(&local)
            .map(|rd| rd.filter(|e| e.is_ok()).count())
            .unwrap_or(0);
        if count > 0 {
            info!(
                partition = partition.start,
                files = count,
                "partition already downloaded, skipping sync"
            );
            return Ok(false);
        }
    }

    std::fs::create_dir_all(&local)?;

    info!(
        partition = partition.start,
        s3 = %s3,
        "syncing partition from S3..."
    );

    let timer = Instant::now();
    let output = tokio::process::Command::new("aws")
        .args([
            "s3",
            "sync",
            &s3,
            local.to_str().unwrap(),
            "--no-sign-request",
            "--quiet",
        ])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "aws s3 sync failed for partition {}: {}",
            partition.start, stderr
        )
        .into());
    }

    let file_count = std::fs::read_dir(&local)
        .map(|rd| rd.filter(|e| e.is_ok()).count())
        .unwrap_or(0);
    let elapsed = timer.elapsed();

    info!(
        partition = partition.start,
        files = file_count,
        elapsed_s = format!("{:.1}", elapsed.as_secs_f64()),
        "partition sync complete"
    );

    Ok(true)
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

/// Check if a ledger already exists in the database.
async fn ledger_exists(pool: &PgPool, sequence: u32) -> Result<bool, sqlx::Error> {
    let row =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM ledgers WHERE sequence = $1)")
            .bind(sequence as i64)
            .fetch_one(pool)
            .await?;

    Ok(row)
}

struct IndexStats {
    indexed: usize,
    skipped: usize,
}

/// Index ledgers in [range_start, range_end] from a downloaded partition.
async fn index_partition(
    partition: &Partition,
    range_start: u32,
    range_end: u32,
    pool: &PgPool,
    global_timer: &Instant,
    global_indexed: &mut usize,
    cleanup: bool,
) -> Result<IndexStats, Box<dyn std::error::Error>> {
    let mut indexed = 0usize;
    let mut skipped = 0usize;

    // Only process ledgers within the requested range
    let first = range_start.max(partition.start);
    let last = range_end.min(partition.end);

    for ledger in first..=last {
        let path = local_ledger_path(partition, ledger);

        if !path.exists() {
            warn!(ledger, "file not found, skipping");
            skipped += 1;
            continue;
        }

        if ledger_exists(pool, ledger).await? {
            if cleanup {
                let _ = std::fs::remove_file(&path);
            }
            skipped += 1;
            continue;
        }

        let compressed = std::fs::read(&path)?;
        let xdr_bytes = xdr_parser::decompress_zstd(&compressed)?;
        let batch = xdr_parser::deserialize_batch(&xdr_bytes)?;
        for ledger_meta in batch.ledger_close_metas.iter() {
            indexer::handler::process::process_ledger(ledger_meta, pool, None).await?;
        }

        if cleanup {
            let _ = std::fs::remove_file(&path);
        }

        indexed += 1;
        *global_indexed += 1;

        if (*global_indexed).is_multiple_of(100) {
            let elapsed = global_timer.elapsed();
            let avg_ms = elapsed.as_millis() as f64 / *global_indexed as f64;
            info!(
                ledger,
                indexed = *global_indexed,
                avg_ms = format!("{avg_ms:.0}"),
                "indexing progress"
            );
        }
    }

    Ok(IndexStats { indexed, skipped })
}

// ---------------------------------------------------------------------------
// Local DEFAULT partition bootstrap
// ---------------------------------------------------------------------------

/// Create a DEFAULT partition on each partitioned table if one doesn't exist.
/// Idempotent — `CREATE TABLE IF NOT EXISTS` per table.
async fn ensure_local_default_partitions(
    pool: &PgPool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    const TABLES: &[&str] = &[
        "transactions",
        "operations",
        "transaction_participants",
        "soroban_events",
        "soroban_invocations",
        "nft_ownership",
        "liquidity_pool_snapshots",
        "account_balance_history",
    ];
    for table in TABLES {
        let ddl =
            format!("CREATE TABLE IF NOT EXISTS {table}_default PARTITION OF {table} DEFAULT");
        if let Err(err) = sqlx::query(&ddl).execute(pool).await {
            // 42P07 = duplicate_table. If it exists under a slightly different
            // form (e.g. attached range partitions already own the slice), skip.
            let code = match &err {
                sqlx::Error::Database(db) => db.code().map(|c| c.into_owned()),
                _ => None,
            };
            if code.as_deref() != Some("42P07") {
                warn!(table, error = %err, "default-partition bootstrap failed");
            }
        } else {
            info!(table, "local DEFAULT partition ensured");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let args = Args::parse();

    if args.start > args.end {
        error!("--start ({}) must be <= --end ({})", args.start, args.end);
        std::process::exit(1);
    }

    let total_range = (args.end - args.start + 1) as usize;
    let partitions = partitions_for_range(args.start, args.end);
    let start_time = Local::now();

    info!(
        start = args.start,
        end = args.end,
        total_ledgers = total_range,
        partitions = partitions.len(),
        "backfill starting at {}",
        start_time.format("%Y-%m-%d %H:%M:%S")
    );

    let pool = db::pool::create_pool(&args.database_url)?;
    info!("connected to database");

    // Partition-management Lambda is the authoritative partition provisioner
    // in production; it currently covers only 3 of the 8 partitioned tables
    // (see task 0149 Out of Scope). Locally we bootstrap DEFAULT partitions
    // on the remaining tables so backfill-bench can exercise the write-path
    // end-to-end without provisioning every monthly range by hand.
    ensure_local_default_partitions(&pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut total_indexed = 0usize;
    let mut total_skipped = 0usize;
    let mut total_download_secs = 0.0f64;

    // ── Pipeline: download N+1 while indexing N ───────────────────────
    // Download first partition (cold start — must wait)
    info!(
        "=== Downloading first partition ({}) ===",
        partitions[0].start
    );
    let dl_timer = Instant::now();
    sync_partition(&partitions[0])
        .await
        .map_err(|e| e.to_string())?;
    total_download_secs += dl_timer.elapsed().as_secs_f64();

    let idx_timer = Instant::now();

    for (i, partition) in partitions.iter().enumerate() {
        // Start downloading next partition in background
        let next_download = if i + 1 < partitions.len() {
            let next = partitions[i + 1].clone();
            Some(tokio::spawn(async move { sync_partition(&next).await }))
        } else {
            None
        };

        // Index current partition
        info!(
            "=== Indexing partition {} ({}/{}) ===",
            partition.start,
            i + 1,
            partitions.len()
        );

        let stats = index_partition(
            partition,
            args.start,
            args.end,
            &pool,
            &idx_timer,
            &mut total_indexed,
            args.cleanup,
        )
        .await?;

        total_skipped += stats.skipped;

        if args.cleanup {
            let folder = partition.local_folder();
            if let Err(e) = std::fs::remove_dir_all(&folder) {
                warn!(partition = partition.start, error = %e, "failed to remove partition folder");
            }
        }

        info!(
            partition = partition.start,
            indexed = stats.indexed,
            skipped = stats.skipped,
            "partition done"
        );

        // Wait for next partition download before proceeding
        if let Some(handle) = next_download {
            if !handle.is_finished() {
                info!(
                    partition = partitions[i + 1].start,
                    "waiting for next partition download..."
                );
            }
            handle.await?.map_err(|e| e.to_string())?;
        }
    }

    // ── Final report ──────────────────────────────────────────────────
    let end_time = Local::now();
    let idx_elapsed = idx_timer.elapsed();
    let avg_ms = if total_indexed > 0 {
        idx_elapsed.as_millis() as f64 / total_indexed as f64
    } else {
        0.0
    };

    info!("=== Backfill complete ===");
    info!(
        "Started:          {}",
        start_time.format("%Y-%m-%d %H:%M:%S")
    );
    info!("Finished:         {}", end_time.format("%Y-%m-%d %H:%M:%S"));
    info!("Range:            {} - {}", args.start, args.end);
    info!("Partitions:       {}", partitions.len());
    info!("Indexed:          {total_indexed}");
    info!("Skipped:          {total_skipped}");
    info!("Download time:    {:.1}s", total_download_secs);
    info!("Index time:       {:.1}s", idx_elapsed.as_secs_f64());
    info!("Avg per ledger:   {avg_ms:.0} ms (index only)");

    Ok(())
}
