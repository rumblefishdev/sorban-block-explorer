//! Phase 2a — DB↔Horizon API field-level diff harness.
//!
//! Picks N random rows from a target table, fetches the matching Horizon
//! resource, diffs the shared fields, and reports per-field mismatches.
//! Runs against the local explorer DB (read-only) and the public Horizon
//! API (`https://horizon.stellar.org` for mainnet).
//!
//! Today implements `--table ledgers` and `--table transactions`. Other
//! tables (accounts, account_balances_current, assets, liquidity_pools)
//! follow the same template and ship in subsequent commits on this branch.
//!
//! Usage:
//!   DATABASE_URL=postgres://... cargo run -p audit-harness --bin horizon-diff -- \
//!       --table ledgers --sample 50 --concurrency 8

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use clap::{Parser, ValueEnum};
use serde::Deserialize;
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::Semaphore;

const HORIZON_DEFAULT: &str = "https://horizon.stellar.org";

#[derive(Parser)]
#[command(name = "horizon-diff", about)]
struct Cli {
    /// PostgreSQL connection string.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Horizon API base URL. Override for testnet.
    #[arg(long, env = "HORIZON_URL", default_value = HORIZON_DEFAULT)]
    horizon_url: String,

    /// Which table to audit. One per invocation; loop in shell to cover
    /// the whole set.
    #[arg(long, value_enum)]
    table: Table,

    /// Number of random rows to sample per run. Larger = more coverage,
    /// more Horizon calls. Default keeps single-run cost reasonable.
    #[arg(long, default_value_t = 50)]
    sample: usize,

    /// Concurrent in-flight Horizon requests. Horizon publishes a
    /// per-IP rate limit (~3600 req/h burst); 8 is well under that.
    #[arg(long, default_value_t = 8)]
    concurrency: usize,
}

#[derive(Clone, Copy, ValueEnum)]
enum Table {
    /// `ledgers` table — compare hash, closed_at, transaction_count,
    /// base_fee, protocol_version against `/ledgers/:sequence`.
    Ledgers,
    /// `transactions` table — compare hash, ledger, successful,
    /// operation_count, fee_charged, source_account, created_at against
    /// `/transactions/:hash`. Sample is random by hash.
    Transactions,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let db = PgPoolOptions::new()
        .max_connections(2)
        .connect(&cli.database_url)
        .await?;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("soroban-block-explorer/audit-harness/0.1")
        .build()?;

    let report = match cli.table {
        Table::Ledgers => {
            diff_ledgers(&db, &http, &cli.horizon_url, cli.sample, cli.concurrency).await?
        }
        Table::Transactions => {
            diff_transactions(&db, &http, &cli.horizon_url, cli.sample, cli.concurrency).await?
        }
    };

    print_report(&report);
    db.close().await;
    if report.mismatched_rows > 0 {
        std::process::exit(1);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Ledgers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // `sequence` is read from our row, not Horizon's; kept on the
// struct to make a future cross-key sanity check trivial.
struct HorizonLedger {
    hash: String,
    sequence: u32,
    closed_at: DateTime<Utc>,
    // Horizon splits tx count by outcome; our schema stores total. Sum below.
    successful_transaction_count: u32,
    failed_transaction_count: u32,
    base_fee_in_stroops: u32,
    protocol_version: u32,
}

impl HorizonLedger {
    fn total_transaction_count(&self) -> u32 {
        self.successful_transaction_count + self.failed_transaction_count
    }
}

#[derive(Debug)]
struct OurLedger {
    sequence: i64,
    hash_hex: String,
    closed_at: DateTime<Utc>,
    transaction_count: i32,
    base_fee: i64,
    protocol_version: i32,
}

async fn diff_ledgers(
    db: &sqlx::PgPool,
    http: &reqwest::Client,
    horizon_url: &str,
    sample: usize,
    concurrency: usize,
) -> Result<DiffReport, Box<dyn std::error::Error + Send + Sync>> {
    // `TABLESAMPLE` would be more correct on huge tables, but `ORDER BY random()`
    // is fine for sample sizes up to ~1k against the unpartitioned `ledgers`
    // table (which is small relative to the partitioned facts).
    let rows = sqlx::query(
        "SELECT sequence, encode(hash, 'hex') AS hash_hex, closed_at,
                transaction_count, base_fee, protocol_version
         FROM ledgers ORDER BY random() LIMIT $1",
    )
    .bind(sample as i64)
    .fetch_all(db)
    .await?;

    let our: Vec<OurLedger> = rows
        .iter()
        .map(|r| OurLedger {
            sequence: r.get("sequence"),
            hash_hex: r.get("hash_hex"),
            closed_at: r.get("closed_at"),
            transaction_count: r.get("transaction_count"),
            base_fee: r.get("base_fee"),
            protocol_version: r.get("protocol_version"),
        })
        .collect();

    if our.is_empty() {
        return Ok(DiffReport {
            table: "ledgers".into(),
            sampled: 0,
            mismatched_rows: 0,
            field_mismatches: vec![],
            unreachable: 0,
        });
    }

    let sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(our.len());
    for o in our {
        let sem = sem.clone();
        let http = http.clone();
        let url = format!("{}/ledgers/{}", horizon_url, o.sequence);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore");
            // Horizon returns 404 for ledgers it hasn't indexed (rare for
            // mainnet recent history, but possible if our DB has rows that
            // never landed on Horizon — Horizon trims very-old ledgers).
            let resp = http.get(&url).send().await;
            (o, resp)
        }));
    }

    let mut report = DiffReport {
        table: "ledgers".into(),
        sampled: handles.len(),
        mismatched_rows: 0,
        field_mismatches: vec![],
        unreachable: 0,
    };
    for h in handles {
        let (o, resp) = h.await.expect("join");
        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::warn!(seq = o.sequence, status = %r.status(), "horizon non-200");
                report.unreachable += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!(seq = o.sequence, error = %e, "horizon fetch failed");
                report.unreachable += 1;
                continue;
            }
        };
        let theirs: HorizonLedger = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(seq = o.sequence, error = %e, "horizon JSON parse failed");
                report.unreachable += 1;
                continue;
            }
        };

        let mut row_mismatches = Vec::new();
        if o.hash_hex != theirs.hash {
            row_mismatches.push(FieldMismatch {
                key: format!("seq={}", o.sequence),
                field: "hash".into(),
                ours: o.hash_hex.clone(),
                theirs: theirs.hash.clone(),
            });
        }
        if o.closed_at != theirs.closed_at {
            row_mismatches.push(FieldMismatch {
                key: format!("seq={}", o.sequence),
                field: "closed_at".into(),
                ours: o.closed_at.to_rfc3339(),
                theirs: theirs.closed_at.to_rfc3339(),
            });
        }
        if o.transaction_count != theirs.total_transaction_count() as i32 {
            row_mismatches.push(FieldMismatch {
                key: format!("seq={}", o.sequence),
                field: "transaction_count".into(),
                ours: o.transaction_count.to_string(),
                theirs: theirs.total_transaction_count().to_string(),
            });
        }
        if o.base_fee != theirs.base_fee_in_stroops as i64 {
            row_mismatches.push(FieldMismatch {
                key: format!("seq={}", o.sequence),
                field: "base_fee".into(),
                ours: o.base_fee.to_string(),
                theirs: theirs.base_fee_in_stroops.to_string(),
            });
        }
        if o.protocol_version != theirs.protocol_version as i32 {
            row_mismatches.push(FieldMismatch {
                key: format!("seq={}", o.sequence),
                field: "protocol_version".into(),
                ours: o.protocol_version.to_string(),
                theirs: theirs.protocol_version.to_string(),
            });
        }

        if !row_mismatches.is_empty() {
            report.mismatched_rows += 1;
            report.field_mismatches.extend(row_mismatches);
        }
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // hash + paging_token are echo from query, kept for future
// sanity checks (Horizon-side hash equality post-roundtrip).
struct HorizonTransaction {
    hash: String,
    ledger: u32,
    successful: bool,
    operation_count: u32,
    /// Horizon serializes `fee_charged` as a JSON string (BIGINT-shaped). Parse
    /// to i64 in our diff layer.
    fee_charged: String,
    source_account: String,
    /// Present iff this is a FeeBump transaction. Horizon's `fee_account`
    /// field is **always** set (= `source_account` for non-FeeBump), so it
    /// is not a reliable FeeBump marker. The presence of an
    /// `inner_transaction` block is the correct discriminator.
    #[serde(default)]
    inner_transaction: Option<HorizonInnerTransaction>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HorizonInnerTransaction {
    /// SHA-256 of the inner-tx envelope. Equals our `transactions.inner_tx_hash`
    /// when correctly extracted.
    hash: String,
}

#[derive(Debug)]
struct OurTransaction {
    hash_hex: String,
    ledger_sequence: i64,
    successful: bool,
    operation_count: i16,
    fee_charged: i64,
    source_strkey: String,
    created_at: DateTime<Utc>,
    inner_tx_hash: Option<Vec<u8>>,
}

async fn diff_transactions(
    db: &sqlx::PgPool,
    http: &reqwest::Client,
    horizon_url: &str,
    sample: usize,
    concurrency: usize,
) -> Result<DiffReport, Box<dyn std::error::Error + Send + Sync>> {
    // JOIN accounts to surface the source StrKey directly. Random sample.
    // Skip parse_error rows — they have intentionally-degraded fields and
    // would generate noise vs the canonical Horizon view.
    let rows = sqlx::query(
        "SELECT encode(t.hash, 'hex') AS hash_hex,
                t.ledger_sequence,
                t.successful,
                t.operation_count,
                t.fee_charged,
                a.account_id AS source_strkey,
                t.created_at,
                t.inner_tx_hash
         FROM transactions t
         JOIN accounts a ON a.id = t.source_id
         WHERE t.parse_error = false
         ORDER BY random() LIMIT $1",
    )
    .bind(sample as i64)
    .fetch_all(db)
    .await?;

    let our: Vec<OurTransaction> = rows
        .iter()
        .map(|r| OurTransaction {
            hash_hex: r.get("hash_hex"),
            ledger_sequence: r.get("ledger_sequence"),
            successful: r.get("successful"),
            operation_count: r.get("operation_count"),
            fee_charged: r.get("fee_charged"),
            source_strkey: r.get("source_strkey"),
            created_at: r.get("created_at"),
            inner_tx_hash: r.get("inner_tx_hash"),
        })
        .collect();

    if our.is_empty() {
        return Ok(DiffReport {
            table: "transactions".into(),
            sampled: 0,
            mismatched_rows: 0,
            field_mismatches: vec![],
            unreachable: 0,
        });
    }

    let sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(our.len());
    for o in our {
        let sem = sem.clone();
        let http = http.clone();
        let url = format!("{}/transactions/{}", horizon_url, o.hash_hex);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore");
            let resp = http.get(&url).send().await;
            (o, resp)
        }));
    }

    let mut report = DiffReport {
        table: "transactions".into(),
        sampled: handles.len(),
        mismatched_rows: 0,
        field_mismatches: vec![],
        unreachable: 0,
    };
    for h in handles {
        let (o, resp) = h.await.expect("join");
        let key = format!("hash={}", &o.hash_hex[..16]);
        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::warn!(hash = %o.hash_hex, status = %r.status(), "horizon non-200");
                report.unreachable += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!(hash = %o.hash_hex, error = %e, "horizon fetch failed");
                report.unreachable += 1;
                continue;
            }
        };
        let theirs: HorizonTransaction = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(hash = %o.hash_hex, error = %e, "horizon JSON parse failed");
                report.unreachable += 1;
                continue;
            }
        };

        let mut row_mismatches = Vec::new();

        if o.hash_hex != theirs.hash {
            // Should never trigger — we queried by hash. If it does, Horizon
            // returned a different tx, which would be a Horizon bug.
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "hash".into(),
                ours: o.hash_hex.clone(),
                theirs: theirs.hash.clone(),
            });
        }
        if o.ledger_sequence != theirs.ledger as i64 {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "ledger_sequence".into(),
                ours: o.ledger_sequence.to_string(),
                theirs: theirs.ledger.to_string(),
            });
        }
        if o.successful != theirs.successful {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "successful".into(),
                ours: o.successful.to_string(),
                theirs: theirs.successful.to_string(),
            });
        }
        if o.operation_count as u32 != theirs.operation_count {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "operation_count".into(),
                ours: o.operation_count.to_string(),
                theirs: theirs.operation_count.to_string(),
            });
        }
        // Horizon `fee_charged` is a JSON string. Parse defensively.
        match theirs.fee_charged.parse::<i64>() {
            Ok(theirs_fee) if theirs_fee != o.fee_charged => {
                row_mismatches.push(FieldMismatch {
                    key: key.clone(),
                    field: "fee_charged".into(),
                    ours: o.fee_charged.to_string(),
                    theirs: theirs_fee.to_string(),
                });
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, raw = %theirs.fee_charged, "fee_charged parse failed");
            }
        }
        // Source account: Horizon's `source_account` is the OUTER source. For
        // FeeBump tx, that's the fee payer; the inner tx source is hidden in
        // `envelope_xdr`. Our `transactions.source_id` is documented to point
        // at the *inner* tx source per task 0168 (envelope-tx_processing
        // alignment). The right comparison therefore depends on whether this
        // is a FeeBump:
        //   - non-FeeBump: ours.source == theirs.source_account
        //   - FeeBump:     ours.source == theirs.envelope_xdr inner-source
        //                  (not exposed as a flat field; defer to Phase 2c
        //                   archive-XDR re-parse).
        let is_fee_bump = theirs.inner_transaction.is_some();
        if !is_fee_bump && o.source_strkey != theirs.source_account {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "source_account".into(),
                ours: o.source_strkey.clone(),
                theirs: theirs.source_account.clone(),
            });
        }
        if o.created_at != theirs.created_at {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "created_at".into(),
                ours: o.created_at.to_rfc3339(),
                theirs: theirs.created_at.to_rfc3339(),
            });
        }
        // inner_tx_hash should be NULL for non-FeeBump and 32 bytes for
        // FeeBump. Horizon doesn't surface this as a flat field; its
        // presence/absence in our row should correlate with whether
        // `fee_account` is set.
        let our_has_inner = o.inner_tx_hash.is_some();
        if our_has_inner != is_fee_bump {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "inner_tx_hash_presence".into(),
                ours: our_has_inner.to_string(),
                theirs: is_fee_bump.to_string(),
            });
        }

        if !row_mismatches.is_empty() {
            report.mismatched_rows += 1;
            report.field_mismatches.extend(row_mismatches);
        }
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct FieldMismatch {
    key: String,
    field: String,
    ours: String,
    theirs: String,
}

#[derive(Debug)]
struct DiffReport {
    table: String,
    sampled: usize,
    mismatched_rows: usize,
    field_mismatches: Vec<FieldMismatch>,
    unreachable: usize,
}

fn print_report(r: &DiffReport) {
    let now = Utc::now().to_rfc3339();
    println!("# audit-harness Phase 2a — DB ↔ Horizon diff\n");
    println!("**Timestamp:** {now}");
    println!("**Table:** `{}`", r.table);
    println!("**Sampled:** {}", r.sampled);
    println!("**Mismatched rows:** {}", r.mismatched_rows);
    println!("**Unreachable on Horizon:** {}\n", r.unreachable);

    if r.mismatched_rows == 0 && r.unreachable == 0 {
        println!("✓ All sampled rows match Horizon field-for-field.");
        return;
    }

    if r.mismatched_rows > 0 {
        // Roll up by field for the operator-facing summary.
        let mut by_field: std::collections::BTreeMap<&str, usize> = Default::default();
        for fm in &r.field_mismatches {
            *by_field.entry(fm.field.as_str()).or_insert(0) += 1;
        }
        println!("## Mismatch counts per field\n");
        for (field, n) in by_field {
            println!("- `{field}`: {n}");
        }

        println!("\n## Sample (first 10 mismatches)\n");
        println!("| key | field | ours | theirs |");
        println!("| --- | --- | --- | --- |");
        for fm in r.field_mismatches.iter().take(10) {
            println!(
                "| {} | {} | `{}` | `{}` |",
                fm.key, fm.field, fm.ours, fm.theirs
            );
        }
    }
}
