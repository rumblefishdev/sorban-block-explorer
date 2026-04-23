//! `status` subcommand — per-partition report: range / indexed / pending.
//!
//! Single source of truth: the `ledgers` table (ADR 0027). The runner
//! deletes each partition's local folder after it indexes it, so "files
//! on disk" is a transient signal with no long-term diagnostic value —
//! we don't report it here. Output is `println!`-only — no tracing
//! events — because `status` is a point-in-time CLI query, not a
//! debug stream.

use crate::error::BackfillError;
use crate::partition::{Partition, partitions_for_range};
use crate::resume::load_completed;

pub async fn execute(database_url: &str, start: u32, end: u32) -> Result<(), BackfillError> {
    assert!(
        start <= end,
        "invalid range: start ({start}) must be <= end ({end})"
    );

    let pool = db::pool::create_pool(database_url)?;
    let completed = load_completed(&pool, start, end).await?;

    let partitions = partitions_for_range(start, end);
    let mut totals = PartitionCounts::default();

    println!();
    println!(
        "  range  {start}..={end}   ({} partition{})",
        partitions.len(),
        if partitions.len() == 1 { "" } else { "s" },
    );
    println!();
    println!(
        "  {:>10}   {:>15}   {:>9}   {:<20}",
        "partition", "indexed / range", "pending", "progress"
    );
    println!("  {:─<10}   {:─<15}   {:─<9}   {:─<20}", "", "", "", "");

    for p in &partitions {
        let row = partition_row(p, start, end, &completed);
        let pct = percent(row.indexed, row.range_len);
        println!(
            "  {:>10}   {:>15}   {:>9}   {} {:>5.1}%",
            p.start,
            format!("{} / {}", row.indexed, row.range_len),
            row.pending,
            bar(pct, 12),
            pct,
        );
        totals.add(&row);
    }

    let pct = percent(totals.indexed, totals.range_len);
    println!("  {:─<10}   {:─<15}   {:─<9}   {:─<20}", "", "", "", "");
    println!(
        "  {:>10}   {:>15}   {:>9}   {} {:>5.1}%",
        "total",
        format!("{} / {}", totals.indexed, totals.range_len),
        totals.pending,
        bar(pct, 12),
        pct,
    );
    println!();

    Ok(())
}

/// Unicode block-element progress bar of `width` cells. Uses eighth-block
/// partials so small percentage deltas show up — a pure full/empty bar
/// would look identical for 0.1% and 7%.
fn bar(percent: f64, width: usize) -> String {
    const BLOCKS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    let filled_eighths = ((percent / 100.0) * (width as f64) * 8.0).round() as usize;
    let full = filled_eighths / 8;
    let rem = filled_eighths % 8;
    let mut s = String::with_capacity(width * 3 + 2);
    s.push('[');
    for _ in 0..full.min(width) {
        s.push('█');
    }
    if full < width {
        s.push(BLOCKS[rem]);
        for _ in (full + 1)..width {
            s.push(' ');
        }
    }
    s.push(']');
    s
}

/// Counts for either a single partition's clamped slice or a running
/// total across partitions — same shape, same arithmetic.
#[derive(Default)]
struct PartitionCounts {
    range_len: usize,
    indexed: usize,
    pending: usize,
}

impl PartitionCounts {
    fn add(&mut self, other: &PartitionCounts) {
        self.range_len += other.range_len;
        self.indexed += other.indexed;
        self.pending += other.pending;
    }
}

fn percent(numer: usize, denom: usize) -> f64 {
    if denom == 0 {
        0.0
    } else {
        (numer as f64 / denom as f64) * 100.0
    }
}

fn partition_row(
    p: &Partition,
    run_start: u32,
    run_end: u32,
    completed: &std::collections::HashSet<u32>,
) -> PartitionCounts {
    // Clamp to the intersection with the requested range — partitions at
    // either edge may stick out of `[run_start, run_end]`.
    let (first, last) = p.clamped(run_start, run_end);
    let range_len = (last - first + 1) as usize;

    // Iterate the clamped range (<=64k per partition) instead of the
    // whole `completed` set — scanning `completed.iter()` per partition
    // is O(partitions × completed) and blows up on full-history runs
    // (~14M ledgers × hundreds of partitions). Copilot review on PR #111.
    let indexed = (first..=last).filter(|s| completed.contains(s)).count();
    // `indexed` can only count sequences in the range, so this never
    // underflows; `saturating_sub` is defense against future bugs.
    let pending = range_len.saturating_sub(indexed);

    PartitionCounts {
        range_len,
        indexed,
        pending,
    }
}
