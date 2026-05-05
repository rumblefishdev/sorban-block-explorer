//! 100k-row batching helper with SAVEPOINT-per-batch retry semantics
//! per task 0186 §Step 0 + AC #5.
//!
//! Each step file calls `Batcher::ledger_windowed` (or `single`) and
//! the helper materializes the batches, wraps each in a savepoint,
//! retries once on transient failure, and aborts on hard failure.

use sqlx::{Executor, PgConnection};

use crate::error::MergeError;

pub const BATCH_SIZE: i64 = 100_000;

/// Stats for a single table's merge — surfaces what to log + sanity check.
#[derive(Debug, Default)]
pub struct MergeStats {
    pub batches: u32,
    pub rows_affected: u64,
}

/// Run a single SQL statement once, wrapped in a savepoint. Used for
/// tables small enough that a single batch covers them (e.g.
/// `wasm_interface_metadata`). The SQL must be a self-contained
/// `INSERT … SELECT … FROM merge_source.<tab> ON CONFLICT …` — no
/// parameter binding (the source is via FDW, not parameters).
pub async fn single(
    conn: &mut PgConnection,
    table: &str,
    sql: &str,
) -> Result<MergeStats, MergeError> {
    tracing::info!(table, "merging (single batch)");
    let savepoint = "sp_single";
    conn.execute(format!("SAVEPOINT {savepoint}").as_str())
        .await?;
    match conn.execute(sql).await {
        Ok(res) => {
            conn.execute(format!("RELEASE SAVEPOINT {savepoint}").as_str())
                .await?;
            Ok(MergeStats {
                batches: 1,
                rows_affected: res.rows_affected(),
            })
        }
        Err(e) => {
            conn.execute(format!("ROLLBACK TO SAVEPOINT {savepoint}").as_str())
                .await?;
            Err(MergeError::Db(e))
        }
    }
}

/// Batch over a `ledger_sequence` window. The SQL template must contain
/// the literal placeholders `{lo}` and `{hi}` (inclusive bounds) that
/// the helper substitutes per batch.
///
/// Window size is chosen so each batch yields ~`BATCH_SIZE` rows on
/// average — the helper queries source MIN/MAX of the bounding column
/// and divides by an estimated rows-per-ledger from the source side.
/// For tables with `ledger_sequence` columns this gives stable
/// progress; for partitioned tables it also exploits constraint-
/// exclusion (Postgres can prune `*_default` scans by the WHERE bound).
pub async fn ledger_windowed(
    conn: &mut PgConnection,
    table: &str,
    source_table: &str,
    ledger_col: &str,
    sql_template: &str,
) -> Result<MergeStats, MergeError> {
    // For empty source MIN/MAX are NULL — must declare both as Option to
    // avoid sqlx ColumnDecode error before we reach the count==0 short-circuit.
    let bounds: (Option<i64>, Option<i64>, i64) = sqlx::query_as(&format!(
        "SELECT MIN({ledger_col})::bigint, MAX({ledger_col})::bigint, COUNT(*)::bigint
         FROM {source_table}"
    ))
    .fetch_one(&mut *conn)
    .await?;

    if bounds.2 == 0 {
        tracing::info!(table, "merging (no source rows — skipping)");
        return Ok(MergeStats::default());
    }
    let (min, max, count) = (
        bounds.0.expect("count > 0 implies non-NULL MIN"),
        bounds.1.expect("count > 0 implies non-NULL MAX"),
        bounds.2,
    );

    let span = (max - min + 1).max(1);
    let rows_per_ledger = (count as f64 / span as f64).max(1.0);
    let window: i64 = ((BATCH_SIZE as f64 / rows_per_ledger).ceil() as i64).max(1);

    tracing::info!(
        table,
        min,
        max,
        count,
        window,
        "merging (ledger-windowed batches)"
    );

    let mut stats = MergeStats::default();
    let mut lo = min;
    while lo <= max {
        let hi = (lo + window - 1).min(max);
        stats.batches += 1;
        let sp = format!("sp_b{:08}", stats.batches);
        let sql = sql_template
            .replace("{lo}", &lo.to_string())
            .replace("{hi}", &hi.to_string());

        conn.execute(format!("SAVEPOINT {sp}").as_str()).await?;
        match conn.execute(sql.as_str()).await {
            Ok(res) => {
                conn.execute(format!("RELEASE SAVEPOINT {sp}").as_str())
                    .await?;
                stats.rows_affected += res.rows_affected();
            }
            Err(first_err) => {
                conn.execute(format!("ROLLBACK TO SAVEPOINT {sp}").as_str())
                    .await?;
                tracing::warn!(table, batch = stats.batches, error = %first_err, "batch failed — retrying once");
                match conn.execute(sql.as_str()).await {
                    Ok(res) => {
                        conn.execute(format!("RELEASE SAVEPOINT {sp}").as_str())
                            .await?;
                        stats.rows_affected += res.rows_affected();
                    }
                    Err(e) => {
                        conn.execute(format!("ROLLBACK TO SAVEPOINT {sp}").as_str())
                            .await?;
                        return Err(MergeError::Db(e));
                    }
                }
            }
        }
        lo = hi + 1;
    }
    Ok(stats)
}
