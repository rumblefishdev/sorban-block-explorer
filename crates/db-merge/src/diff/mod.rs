//! `merge diff` — per-table normalized natural-key hash comparison
//! between two databases. Per task 0186 §Step 4 + AC #10:
//!
//! For each table we run a SQL projection that:
//! - replaces every surrogate FK with the natural key of the referenced
//!   row (so two DBs with identical logical content but different
//!   BIGSERIAL allocations produce the same hash);
//! - excludes the surrogate `id` column itself;
//! - excludes `search_vector` (GENERATED ALWAYS — recomputed at insert);
//! - sorts deterministically by natural key;
//! - aggregates `md5(string_agg(canonical, '|' ORDER BY sort_key))`.
//!
//! Output is a 17-row table: `table | rows_left | rows_right | hash_left
//! | hash_right | match`. Exit code 1 if any mismatch.

use sqlx::{Connection, PgConnection, Row};

use crate::error::MergeError;

pub mod account_balances_current;
pub mod accounts;
pub mod assets;
pub mod ledgers;
pub mod liquidity_pool_snapshots;
pub mod liquidity_pools;
pub mod lp_positions;
pub mod nft_ownership;
pub mod nfts;
pub mod operations_appearances;
pub mod soroban_contracts;
pub mod soroban_events_appearances;
pub mod soroban_invocations_appearances;
pub mod transaction_hash_index;
pub mod transaction_participants;
pub mod transactions;
pub mod wasm_interface_metadata;

/// Tables to diff, in topological order matching the merge steps so the
/// report is easy to read alongside ingest logs.
pub const TABLES: &[(&str, &str)] = &[
    ("ledgers", ledgers::SQL),
    ("accounts", accounts::SQL),
    ("wasm_interface_metadata", wasm_interface_metadata::SQL),
    ("soroban_contracts", soroban_contracts::SQL),
    ("assets", assets::SQL),
    ("liquidity_pools", liquidity_pools::SQL),
    ("nfts", nfts::SQL),
    ("transactions", transactions::SQL),
    ("transaction_hash_index", transaction_hash_index::SQL),
    ("operations_appearances", operations_appearances::SQL),
    ("transaction_participants", transaction_participants::SQL),
    (
        "soroban_events_appearances",
        soroban_events_appearances::SQL,
    ),
    (
        "soroban_invocations_appearances",
        soroban_invocations_appearances::SQL,
    ),
    ("nft_ownership", nft_ownership::SQL),
    ("liquidity_pool_snapshots", liquidity_pool_snapshots::SQL),
    ("lp_positions", lp_positions::SQL),
    ("account_balances_current", account_balances_current::SQL),
];

#[derive(Debug)]
struct Row1 {
    table: &'static str,
    rows_left: i64,
    rows_right: i64,
    hash_left: Option<String>,
    hash_right: Option<String>,
}

impl Row1 {
    fn matches(&self) -> bool {
        self.rows_left == self.rows_right && self.hash_left == self.hash_right
    }
}

pub async fn execute(left_url: &str, right_url: &str) -> Result<(), MergeError> {
    let mut left = PgConnection::connect(left_url).await?;
    let mut right = PgConnection::connect(right_url).await?;

    let mut report: Vec<Row1> = Vec::with_capacity(TABLES.len());
    for (table, sql) in TABLES {
        let (lh, lc) = run_projection(&mut left, sql).await?;
        let (rh, rc) = run_projection(&mut right, sql).await?;
        report.push(Row1 {
            table,
            rows_left: lc,
            rows_right: rc,
            hash_left: lh,
            hash_right: rh,
        });
    }

    print_report(&report);
    let mismatched: Vec<&str> = report
        .iter()
        .filter(|r| !r.matches())
        .map(|r| r.table)
        .collect();
    if mismatched.is_empty() {
        println!("\n{}/{} tables match", report.len(), report.len());
    } else {
        println!(
            "\n{}/{} tables match — {} mismatches: {}",
            report.len() - mismatched.len(),
            report.len(),
            mismatched.len(),
            mismatched.join(", ")
        );
        std::process::exit(1);
    }

    Ok(())
}

async fn run_projection(
    conn: &mut PgConnection,
    sql: &str,
) -> Result<(Option<String>, i64), MergeError> {
    let row = sqlx::query(sql).fetch_one(conn).await?;
    let hash: Option<String> = row.try_get("hash")?;
    let rows: i64 = row.try_get("rows")?;
    Ok((hash, rows))
}

fn print_report(report: &[Row1]) {
    let table_w = report
        .iter()
        .map(|r| r.table.len())
        .max()
        .unwrap_or(20)
        .max(20);
    println!(
        "{:<table_w$}  {:>8}  {:>8}  {:<10}  {:<10}  MATCH",
        "TABLE", "ROWS_L", "ROWS_R", "HASH_L", "HASH_R"
    );
    println!(
        "{}",
        "-".repeat(table_w + 2 + 8 + 2 + 8 + 2 + 10 + 2 + 10 + 2 + 5)
    );
    for r in report {
        let lh = r
            .hash_left
            .as_deref()
            .map(|h| &h[..8.min(h.len())])
            .unwrap_or("(empty)");
        let rh = r
            .hash_right
            .as_deref()
            .map(|h| &h[..8.min(h.len())])
            .unwrap_or("(empty)");
        let mark = if r.matches() { "OK" } else { "FAIL" };
        println!(
            "{:<table_w$}  {:>8}  {:>8}  {:<10}  {:<10}  {}",
            r.table, r.rows_left, r.rows_right, lh, rh, mark
        );
    }
}
