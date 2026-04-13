//! Local backfill benchmark — pipelined download + index.
//!
//! A downloader task fetches XDR files from Stellar's public S3 bucket via
//! AWS CLI and streams completed ledger numbers through a bounded channel.
//! An indexer task reads from the channel, decompresses, persists to Postgres,
//! and deletes the file. Backpressure is automatic: when the channel buffer
//! (default 20) is full, the downloader pauses until the indexer catches up.
//!
//! Usage:
//!   cargo run -p backfill-bench -- --start 62015000 --end 62015999

use aws_sdk_cloudwatch::Client as CloudWatchClient;
use chrono::Local;
use clap::Parser;
use sqlx::PgPool;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use tracing::{error, info, warn};

const S3_BUCKET_BASE: &str = "s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet";
const PARTITION_SIZE: u32 = 64000;
const TEMP_DIR: &str = ".temp";
const CONCURRENT_DOWNLOADS: usize = 20;
const CHANNEL_BUFFER: usize = 20;

#[derive(Parser)]
#[command(name = "backfill-bench", about = "Local backfill benchmark")]
struct Args {
    /// First ledger to index (inclusive)
    #[arg(long)]
    start: u32,

    /// Last ledger to index (inclusive)
    #[arg(long)]
    end: u32,

    /// PostgreSQL connection string (default: DATABASE_URL env var)
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
}

/// Build the S3 URI for a single ledger file.
fn s3_uri(ledger: u32) -> String {
    let partition_start = ledger - (ledger % PARTITION_SIZE);
    let partition_end = partition_start + PARTITION_SIZE - 1;
    let partition_hex = format!("{:08X}", u32::MAX - partition_start);
    let file_hex = format!("{:08X}", u32::MAX - ledger);

    format!(
        "{S3_BUCKET_BASE}/{partition_hex}--{partition_start}-{partition_end}/{file_hex}--{ledger}.xdr.zst"
    )
}

/// Local path for a downloaded ledger file.
fn local_path(ledger: u32) -> PathBuf {
    Path::new(TEMP_DIR).join(format!("{ledger}.xdr.zst"))
}

/// Check if a ledger already exists in the database.
async fn ledger_exists(pool: &PgPool, sequence: u32) -> Result<bool, sqlx::Error> {
    let row =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM ledgers WHERE sequence = $1)")
            .bind(sequence as i64)
            .fetch_one(pool)
            .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Downloader task
// ---------------------------------------------------------------------------

async fn downloader(
    start: u32,
    end: u32,
    tx: mpsc::Sender<(u32, bool)>,
    stats: Arc<DownloadStats>,
) {
    std::fs::create_dir_all(TEMP_DIR).expect("failed to create .temp directory");

    let semaphore = Arc::new(Semaphore::new(CONCURRENT_DOWNLOADS));
    let mut tasks = JoinSet::new();

    for ledger in start..=end {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore closed");
        let tx = tx.clone();
        let stats = stats.clone();

        tasks.spawn(async move {
            let uri = s3_uri(ledger);
            let dest = local_path(ledger);

            let output = tokio::process::Command::new("aws")
                .args([
                    "s3",
                    "cp",
                    &uri,
                    dest.to_str().unwrap(),
                    "--no-sign-request",
                    "--quiet",
                ])
                .output()
                .await;

            match output {
                Ok(o) if o.status.success() => {
                    let file_size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
                    stats.bytes.fetch_add(file_size, Ordering::Relaxed);
                    stats.downloaded.fetch_add(1, Ordering::Relaxed);

                    // Send to indexer — blocks if channel is full (backpressure).
                    // Permit held until send completes, so new downloads are blocked
                    // when the indexer can't keep up — limits files on disk.
                    let _ = tx.send((ledger, true)).await;
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    warn!(ledger, %stderr, "download failed");
                    stats.failed.fetch_add(1, Ordering::Relaxed);
                    let _ = tx.send((ledger, false)).await;
                }
                Err(e) => {
                    warn!(ledger, error = %e, "failed to spawn aws cli");
                    stats.failed.fetch_add(1, Ordering::Relaxed);
                    let _ = tx.send((ledger, false)).await;
                }
            }

            // Release permit AFTER channel send — this couples download rate
            // to indexer consumption, preventing unbounded file accumulation.
            drop(permit);
        });
    }

    // Await tasks as they complete — JoinSet frees handles immediately,
    // no unbounded Vec<JoinHandle> allocation for large ranges.
    while let Some(result) = tasks.join_next().await {
        if let Err(e) = result {
            warn!(error = %e, "download task panicked");
        }
    }

    // tx is dropped here, closing the channel → indexer will finish
}

struct DownloadStats {
    downloaded: AtomicUsize,
    failed: AtomicUsize,
    bytes: AtomicU64,
}

// ---------------------------------------------------------------------------
// Indexer task
// ---------------------------------------------------------------------------

async fn indexer(
    mut rx: mpsc::Receiver<(u32, bool)>,
    pool: &PgPool,
    cw_client: &CloudWatchClient,
    start: u32,
) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    let timer = Instant::now();
    let mut indexed = 0usize;
    let mut skipped = 0usize;

    // Buffer out-of-order arrivals and process in ascending sequence order.
    // Maps ledger number → downloaded successfully.
    let mut next_expected = start;
    let mut ready_buf = BTreeMap::new();
    let mut channel_open = true;

    loop {
        // Drain the ready buffer: process all consecutive ledgers from next_expected
        while let Some(entry) = ready_buf.first_entry() {
            if *entry.key() != next_expected {
                break; // gap — wait for missing ledger
            }
            let ledger = *entry.key();
            let success: bool = entry.remove();
            next_expected += 1;

            if !success {
                skipped += 1;
                continue;
            }

            let path = local_path(ledger);

            if ledger_exists(pool, ledger).await? {
                let _ = std::fs::remove_file(&path);
                skipped += 1;
                continue;
            }

            let compressed = std::fs::read(&path)?;
            let xdr_bytes = xdr_parser::decompress_zstd(&compressed)?;
            let batch = xdr_parser::deserialize_batch(&xdr_bytes)?;
            for ledger_meta in batch.ledger_close_metas.iter() {
                indexer::handler::process::process_ledger(ledger_meta, pool, cw_client).await?;
            }

            let _ = std::fs::remove_file(&path);
            indexed += 1;

            if indexed.is_multiple_of(10) {
                let elapsed = timer.elapsed();
                let avg_ms = elapsed.as_millis() as f64 / indexed as f64;
                info!(
                    ledger,
                    indexed,
                    skipped,
                    avg_ms = format!("{avg_ms:.0}"),
                    "indexing progress"
                );
            }
        }

        if !channel_open && ready_buf.is_empty() {
            break;
        }

        // Wait for next downloaded ledger
        match rx.recv().await {
            Some((ledger, success)) => {
                ready_buf.insert(ledger, success);
            }
            None => {
                channel_open = false;
            }
        }
    }

    Ok((indexed, skipped))
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
    let start_time = Local::now();
    let timer = Instant::now();

    info!(
        start = args.start,
        end = args.end,
        total = total_range,
        concurrent_downloads = CONCURRENT_DOWNLOADS,
        channel_buffer = CHANNEL_BUFFER,
        "backfill pipeline starting at {}",
        start_time.format("%Y-%m-%d %H:%M:%S")
    );

    let pool = db::pool::create_pool(&args.database_url)?;
    info!("connected to database");

    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let cw_client = CloudWatchClient::new(&aws_config);

    // Shared download stats
    let dl_stats = Arc::new(DownloadStats {
        downloaded: AtomicUsize::new(0),
        failed: AtomicUsize::new(0),
        bytes: AtomicU64::new(0),
    });

    // Channel: downloader → indexer
    let (tx, rx) = mpsc::channel(CHANNEL_BUFFER);

    // Spawn downloader in background
    let dl_stats_clone = dl_stats.clone();
    let dl_handle = tokio::spawn(async move {
        downloader(args.start, args.end, tx, dl_stats_clone).await;
    });

    // Run indexer on main task
    let (indexed, skipped) = indexer(rx, &pool, &cw_client, args.start).await?;

    // Wait for downloader to fully finish
    dl_handle.await?;

    // ── Final report ───────────────────────────────────────────────────
    let end_time = Local::now();
    let total_elapsed = timer.elapsed();
    let downloaded = dl_stats.downloaded.load(Ordering::Relaxed);
    let dl_failed = dl_stats.failed.load(Ordering::Relaxed);
    let total_bytes = dl_stats.bytes.load(Ordering::Relaxed);

    let avg_ms = if indexed > 0 {
        total_elapsed.as_millis() as f64 / indexed as f64
    } else {
        0.0
    };
    let avg_file_kb = if downloaded > 0 {
        total_bytes as f64 / downloaded as f64 / 1024.0
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
    info!("Downloaded:       {downloaded} (failed: {dl_failed})");
    info!("Indexed:          {indexed}");
    info!("Skipped:          {skipped}");
    info!("Total time:       {:.1}s", total_elapsed.as_secs_f64());
    info!("Avg per ledger:   {avg_ms:.0} ms (wall clock)");
    info!("Avg file size:    {avg_file_kb:.1} KB");

    Ok(())
}
