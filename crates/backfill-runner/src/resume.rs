//! DB-only resume: which sequences in `[start, end]` are already in `ledgers`?
//!
//! Single batch query at startup — no watermark file, no side-channel state.
//! The `ledgers` table (ADR 0027) is the single source of truth.

use sqlx::PgPool;
use std::collections::HashSet;
use tracing::info;

use crate::error::BackfillError;

/// Load sequences already present in `ledgers` within `[start, end]`.
pub async fn load_completed(
    pool: &PgPool,
    start: u32,
    end: u32,
) -> Result<HashSet<u32>, BackfillError> {
    let rows: Vec<i64> =
        sqlx::query_scalar("SELECT sequence FROM ledgers WHERE sequence BETWEEN $1 AND $2")
            .bind(start as i64)
            .bind(end as i64)
            .fetch_all(pool)
            .await?;

    let set: HashSet<u32> = rows.into_iter().map(|s| s as u32).collect();
    info!(
        start,
        end,
        completed = set.len(),
        total = end - start + 1,
        "resume state loaded"
    );
    Ok(set)
}
