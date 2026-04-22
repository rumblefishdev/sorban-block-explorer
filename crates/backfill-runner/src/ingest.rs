//! Parse + persist a single ledger from a local file, and drive the
//! sequential indexing of a whole partition. Thin glue over existing
//! crates — all write-path logic lives in
//! `indexer::handler::process::process_ledger`.
//!
//! The caller is responsible for producing the files on disk (via
//! `aws s3 sync` — see the `sync` module).

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use indexer::handler::HandlerError;
use sqlx::PgPool;
use tracing::info;

use crate::error::BackfillError;
use crate::partition::{Partition, local_ledger_path};

/// Per-ledger parse + persist timings. Decompression isn't timed —
/// it's deterministic work on a fixed input and not a useful diagnostic
/// signal relative to parse/persist (task 0145 decision).
#[derive(Debug, Clone, Copy, Default)]
pub struct LedgerTimings {
    pub bytes: usize,
    pub parse_ms: u128,
    pub persist_ms: u128,
}

impl LedgerTimings {
    /// Total time attributable to this ledger (parse + persist). Used
    /// for min/max aggregation at the partition / run level.
    pub fn total_ms(&self) -> u128 {
        self.parse_ms + self.persist_ms
    }
}

/// Aggregate produced by `index_partition`. Powers the partition-end
/// log line and the run-level summary. Missing / failed ledgers are
/// **not** tracked — both panic (task 0145 debug-first stance), so the
/// only non-indexed bucket is "already in DB, skipped".
#[derive(Debug, Clone, Default)]
pub struct PartitionStats {
    pub indexed: usize,
    pub skipped_completed: usize,
    pub total_bytes: u64,
    pub parse_total_ms: u128,
    pub persist_total_ms: u128,
    /// Min / max per-ledger total_ms (parse + persist). `None` when the
    /// partition indexed zero ledgers (all already in DB, or empty).
    pub min_ledger_ms: Option<u128>,
    pub max_ledger_ms: Option<u128>,
    pub wall_clock: Duration,
}

/// Read, decompress, deserialize, and persist a single ledger file.
///
/// `partition_start` is passed in explicitly so the structured log event
/// carries enough context to answer "which partition owned this ledger?"
/// without re-parsing the filename.
pub async fn ingest_ledger_from_file(
    path: &Path,
    pool: &PgPool,
    seq: u32,
    partition_start: u32,
) -> Result<LedgerTimings, BackfillError> {
    let compressed = tokio::fs::read(path).await?;
    let bytes = compressed.len();

    let xdr_bytes = xdr_parser::decompress_zstd(&compressed).map_err(HandlerError::from)?;

    let parse_start = Instant::now();
    let batch = xdr_parser::deserialize_batch(&xdr_bytes).map_err(HandlerError::from)?;
    let parse_ms = parse_start.elapsed().as_millis();

    let persist_start = Instant::now();
    for meta in batch.ledger_close_metas.iter() {
        indexer::handler::process::process_ledger(meta, pool, None).await?;
    }
    let persist_ms = persist_start.elapsed().as_millis();

    info!(
        seq,
        partition = partition_start,
        bytes,
        parse_ms,
        persist_ms,
        "ledger ingested"
    );

    Ok(LedgerTimings {
        bytes,
        parse_ms,
        persist_ms,
    })
}

/// Sequentially index all ledgers in `partition` that fall within
/// `[range_start, range_end]`, skipping any sequence already present in
/// `completed`.
///
/// Assumes the partition has been synced to disk. A missing file here
/// means sync produced an incomplete dir (rare archive hole or a sync
/// bug) — we panic rather than warn-and-continue, per task 0145's
/// debug-first stance. Parse / persist errors similarly propagate and
/// panic at the top-level.
///
/// Emits `partition indexing started` / `partition indexing complete`
/// at info level when `--verbose` is on.
pub async fn index_partition(
    partition: &Partition,
    temp_dir: &Path,
    pool: &PgPool,
    range_start: u32,
    range_end: u32,
    completed: &HashSet<u32>,
) -> Result<PartitionStats, BackfillError> {
    let (first, last) = partition.clamped(range_start, range_end);

    info!(
        partition = partition.start,
        first, last, "partition indexing started"
    );

    let wall_start = Instant::now();
    let mut stats = PartitionStats::default();

    for seq in first..=last {
        if completed.contains(&seq) {
            stats.skipped_completed += 1;
            continue;
        }
        let path = local_ledger_path(partition, seq, temp_dir);
        assert!(
            path.exists(),
            "ledger file missing post-sync: partition={} seq={} path={}",
            partition.start,
            seq,
            path.display()
        );
        let t = ingest_ledger_from_file(&path, pool, seq, partition.start).await?;
        stats.indexed += 1;
        stats.total_bytes += t.bytes as u64;
        stats.parse_total_ms += t.parse_ms;
        stats.persist_total_ms += t.persist_ms;

        let ledger_ms = t.total_ms();
        stats.min_ledger_ms = Some(stats.min_ledger_ms.map_or(ledger_ms, |m| m.min(ledger_ms)));
        stats.max_ledger_ms = Some(stats.max_ledger_ms.map_or(ledger_ms, |m| m.max(ledger_ms)));
    }

    stats.wall_clock = wall_start.elapsed();
    let wall_s = stats.wall_clock.as_secs_f64().max(0.001);
    let throughput = stats.indexed as f64 / wall_s;

    info!(
        partition = partition.start,
        indexed = stats.indexed,
        skipped_completed = stats.skipped_completed,
        total_bytes = stats.total_bytes,
        parse_total_ms = stats.parse_total_ms,
        persist_total_ms = stats.persist_total_ms,
        min_ledger_ms = stats.min_ledger_ms.unwrap_or(0),
        max_ledger_ms = stats.max_ledger_ms.unwrap_or(0),
        wall_clock_secs = format!("{:.1}", wall_s),
        throughput = format!("{:.2} ledgers/s", throughput),
        "partition indexing complete"
    );

    Ok(stats)
}
