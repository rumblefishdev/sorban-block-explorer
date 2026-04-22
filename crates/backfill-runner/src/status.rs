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
    let mut totals = Totals::default();
    let mut fully_done = 0usize;

    println!("range: {start}..={end}   partitions: {}", partitions.len());
    println!(
        "{:>12} {:>21} {:>10} {:>10}",
        "partition", "indexed / range", "pending", "progress"
    );

    for p in &partitions {
        let row = partition_row(p, start, end, &completed);
        if row.pending == 0 && row.range_len > 0 {
            fully_done += 1;
        }
        println!(
            "{:>12} {:>21} {:>10} {:>9.1}%",
            p.start,
            format!("{} / {}", row.indexed, row.range_len),
            row.pending,
            percent(row.indexed, row.range_len),
        );
        totals.add(&row);
    }

    println!("{:-<58}", "");
    println!(
        "{:>12} {:>21} {:>10} {:>9.1}%",
        "total",
        format!("{} / {}", totals.indexed, totals.range_len),
        totals.pending,
        percent(totals.indexed, totals.range_len),
    );

    println!();
    println!("=== summary ===");
    println!(
        "partitions fully indexed: {} / {}",
        fully_done,
        partitions.len()
    );
    println!(
        "ledgers indexed:          {} / {}  ({:.1}%)",
        totals.indexed,
        totals.range_len,
        percent(totals.indexed, totals.range_len)
    );
    println!("ledgers pending:          {}", totals.pending);

    Ok(())
}

#[derive(Default)]
struct Row {
    range_len: usize,
    indexed: usize,
    pending: usize,
}

#[derive(Default)]
struct Totals {
    range_len: usize,
    indexed: usize,
    pending: usize,
}

impl Totals {
    fn add(&mut self, r: &Row) {
        self.range_len += r.range_len;
        self.indexed += r.indexed;
        self.pending += r.pending;
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
) -> Row {
    // Clamp to the intersection with the requested range — partitions at
    // either edge may stick out of `[run_start, run_end]`.
    let (first, last) = p.clamped(run_start, run_end);
    let range_len = (last - first + 1) as usize;

    let indexed = completed
        .iter()
        .filter(|s| **s >= first && **s <= last)
        .count();
    // `indexed` can only count sequences in the range, so this never
    // underflows; `saturating_sub` is defense against future bugs.
    let pending = range_len.saturating_sub(indexed);

    Row {
        range_len,
        indexed,
        pending,
    }
}
