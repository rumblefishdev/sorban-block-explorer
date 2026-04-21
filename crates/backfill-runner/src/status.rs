//! `status` subcommand — reports ingested / missing ledgers for a range.
//!
//! Single source of truth: the `ledgers` table. No separate state store.

use tracing::info;

use crate::error::BackfillError;
use crate::resume;

pub async fn execute(database_url: &str, start: u32, end: u32) -> Result<(), BackfillError> {
    assert!(start <= end, "--start must be <= --end");

    let pool = db::pool::create_pool(database_url)?;
    let completed = resume::load_completed(&pool, start, end).await?;

    let total = (end - start + 1) as usize;
    let ingested = completed.len();
    let missing_count = total - ingested;

    let mut missing: Vec<u32> = (start..=end).filter(|s| !completed.contains(s)).collect();
    missing.sort_unstable();

    info!(
        start,
        end,
        total,
        ingested,
        missing = missing_count,
        "status report"
    );

    println!("range:     {}..={}", start, end);
    println!("total:     {}", total);
    println!("ingested:  {}", ingested);
    println!("missing:   {}", missing_count);

    if !missing.is_empty() {
        println!("missing sequences:");
        for seq in &missing {
            println!("  {}", seq);
        }
    }

    Ok(())
}
