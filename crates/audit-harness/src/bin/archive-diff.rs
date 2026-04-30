//! Phase 2c — DB ↔ raw archive XDR re-parse.
//!
//! Picks N random rows from a target table, fetches the matching
//! LedgerCloseMeta XDR file from the public Stellar archive
//! (`s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`), parses
//! it with `stellar-xdr` directly (NOT through `crates/xdr-parser`),
//! and diffs the canonical fields against the DB row.
//!
//! Independence is the point — if our `xdr-parser` extracts a wrong
//! value, this harness uses the *raw XDR shape* as the ground truth
//! and surfaces the discrepancy. This is what caught task 0176
//! (`SHA256(LedgerHeaderHistoryEntry)` vs `header_entry.hash`) on the
//! first run, scaled to all rows.
//!
//! Today implements `--table ledgers` only — same starting point as
//! `horizon-diff`. Other tables follow the same shape: random sample
//! → S3 fetch → independent parse → field diff.
//!
//! Usage:
//!   DATABASE_URL=postgres://... cargo run -p audit-harness --bin archive-diff -- \
//!       --table ledgers --sample 50 --concurrency 4

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use clap::{Parser, ValueEnum};
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use stellar_xdr::curr::{LedgerCloseMeta, LedgerCloseMetaBatch, Limits, ReadXdr};
use tokio::sync::Semaphore;

const PARTITION_SIZE: u32 = 64_000;
const ARCHIVE_BASE: &str =
    "https://aws-public-blockchain.s3.amazonaws.com/v1.1/stellar/ledgers/pubnet";

#[derive(Parser)]
#[command(name = "archive-diff", about)]
struct Cli {
    /// PostgreSQL connection string.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Public archive base URL. Override only if a private mirror is
    /// available; default is the SDF-published S3 bucket.
    #[arg(long, env = "ARCHIVE_URL", default_value = ARCHIVE_BASE)]
    archive_url: String,

    /// Which table to audit. One per invocation.
    #[arg(long, value_enum)]
    table: Table,

    /// Number of random rows to sample. Each sample = 1 S3 GET + 1
    /// zstd decompress + 1 XDR deserialize. Larger = more network +
    /// CPU; default keeps single-run wall clock under a minute.
    #[arg(long, default_value_t = 50)]
    sample: usize,

    /// Concurrent in-flight S3 fetches. S3 doesn't rate-limit at this
    /// scale, but local CPU saturates at ~4 parallel decompress + parse.
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
}

#[derive(Clone, Copy, ValueEnum)]
enum Table {
    /// `ledgers` table — re-parse XDR header + canonical hash, diff
    /// against `ledgers.hash / closed_at / protocol_version /
    /// transaction_count / base_fee`.
    Ledgers,
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
        .timeout(Duration::from_secs(30))
        .user_agent("soroban-block-explorer/audit-harness/0.1")
        .build()?;

    let report = match cli.table {
        Table::Ledgers => {
            diff_ledgers(&db, &http, &cli.archive_url, cli.sample, cli.concurrency).await?
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
// Ledger-file URL construction
// ---------------------------------------------------------------------------

/// Path component for a partition (64k-aligned) in the SDF archive layout.
/// Format: `{HEX(u32::MAX - p_start)}--{p_start}-{p_end}/`.
fn partition_dir(p_start: u32) -> String {
    let p_end = p_start + PARTITION_SIZE - 1;
    format!("{:08X}--{}-{}", u32::MAX - p_start, p_start, p_end)
}

/// File name for a single ledger.
fn ledger_file(seq: u32) -> String {
    format!("{:08X}--{}.xdr.zst", u32::MAX - seq, seq)
}

fn ledger_url(base: &str, seq: u32) -> String {
    let p_start = seq - (seq % PARTITION_SIZE);
    format!("{}/{}/{}", base, partition_dir(p_start), ledger_file(seq))
}

// ---------------------------------------------------------------------------
// XDR re-parse
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CanonicalLedger {
    sequence: u32,
    hash_hex: String,
    closed_at: DateTime<Utc>,
    protocol_version: u32,
    transaction_count: u32,
    base_fee: u32,
}

/// Independent extractor — does NOT call into `crates/xdr-parser`. Reads
/// the same fields the canonical Stellar ledger hash + protocol guarantee.
fn canonical_fields(meta: &LedgerCloseMeta) -> CanonicalLedger {
    let header_entry = match meta {
        LedgerCloseMeta::V0(v) => &v.ledger_header,
        LedgerCloseMeta::V1(v) => &v.ledger_header,
        LedgerCloseMeta::V2(v) => &v.ledger_header,
    };
    let header = &header_entry.header;
    let tx_count = match meta {
        LedgerCloseMeta::V0(v) => v.tx_processing.len() as u32,
        LedgerCloseMeta::V1(v) => v.tx_processing.len() as u32,
        LedgerCloseMeta::V2(v) => v.tx_processing.len() as u32,
    };
    CanonicalLedger {
        sequence: header.ledger_seq,
        // CANONICAL hash — read directly from the LedgerHeaderHistoryEntry.
        // This is the field 0176 should reference; comparing here against
        // our DB hash exposes the bug at scale.
        hash_hex: hex::encode(header_entry.hash.0),
        closed_at: Utc
            .timestamp_opt(header.scp_value.close_time.0 as i64, 0)
            .single()
            .expect("valid unix timestamp"),
        protocol_version: header.ledger_version,
        transaction_count: tx_count,
        base_fee: header.base_fee,
    }
}

async fn fetch_and_parse_ledger(
    http: &reqwest::Client,
    archive_url: &str,
    seq: u32,
) -> Result<CanonicalLedger, Box<dyn std::error::Error + Send + Sync>> {
    let url = ledger_url(archive_url, seq);
    let bytes = http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let xdr = zstd::decode_all(&bytes[..])?;
    // The archive ships ledgers as a `LedgerCloseMetaBatch` (containing one
    // or more `LedgerCloseMeta`), not a bare `LedgerCloseMeta`. Each file
    // typically wraps a single ledger but the batch shape is the on-the-wire
    // contract.
    //
    // `Limits::none()` because the public archive is trusted and ledger
    // files routinely exceed any conservative depth/length cap from XDR
    // safety budgets. Indexer-side parsing applies real limits; this
    // harness does not.
    let batch = LedgerCloseMetaBatch::from_xdr(&xdr, Limits::none())?;
    let meta = batch
        .ledger_close_metas
        .iter()
        .find(|m| {
            let header_seq = match m {
                LedgerCloseMeta::V0(v) => v.ledger_header.header.ledger_seq,
                LedgerCloseMeta::V1(v) => v.ledger_header.header.ledger_seq,
                LedgerCloseMeta::V2(v) => v.ledger_header.header.ledger_seq,
            };
            header_seq == seq
        })
        .ok_or("requested ledger seq not in batch")?;
    Ok(canonical_fields(meta))
}

// ---------------------------------------------------------------------------
// Ledgers diff
// ---------------------------------------------------------------------------

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
    archive_url: &str,
    sample: usize,
    concurrency: usize,
) -> Result<DiffReport, Box<dyn std::error::Error + Send + Sync>> {
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
        return Ok(empty_report("ledgers"));
    }

    let sem = Arc::new(Semaphore::new(concurrency));
    let archive = Arc::new(archive_url.to_string());
    let mut handles = Vec::with_capacity(our.len());
    for o in our {
        let sem = sem.clone();
        let http = http.clone();
        let archive = archive.clone();
        let seq = u32::try_from(o.sequence).expect("ledger seq fits u32");
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore");
            let res = fetch_and_parse_ledger(&http, &archive, seq).await;
            (o, res)
        }));
    }

    let mut report = empty_report("ledgers");
    report.sampled = handles.len();
    for h in handles {
        let (o, res) = h.await.expect("join");
        let key = format!("seq={}", o.sequence);
        let theirs = match res {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(seq = o.sequence, error = %e, "fetch/parse failed");
                report.unreachable += 1;
                continue;
            }
        };

        let mut row_mismatches = Vec::new();
        if o.hash_hex != theirs.hash_hex {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "hash".into(),
                ours: o.hash_hex.clone(),
                theirs: theirs.hash_hex.clone(),
            });
        }
        if o.closed_at != theirs.closed_at {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "closed_at".into(),
                ours: o.closed_at.to_rfc3339(),
                theirs: theirs.closed_at.to_rfc3339(),
            });
        }
        if o.transaction_count != theirs.transaction_count as i32 {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "transaction_count".into(),
                ours: o.transaction_count.to_string(),
                theirs: theirs.transaction_count.to_string(),
            });
        }
        if o.base_fee != theirs.base_fee as i64 {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "base_fee".into(),
                ours: o.base_fee.to_string(),
                theirs: theirs.base_fee.to_string(),
            });
        }
        if o.protocol_version != theirs.protocol_version as i32 {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "protocol_version".into(),
                ours: o.protocol_version.to_string(),
                theirs: theirs.protocol_version.to_string(),
            });
        }
        if u32::try_from(o.sequence).unwrap_or(0) != theirs.sequence {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "sequence".into(),
                ours: o.sequence.to_string(),
                theirs: theirs.sequence.to_string(),
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
// Reporting (mirrors horizon-diff's shape)
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

fn empty_report(table: &str) -> DiffReport {
    DiffReport {
        table: table.to_string(),
        sampled: 0,
        mismatched_rows: 0,
        field_mismatches: vec![],
        unreachable: 0,
    }
}

fn print_report(r: &DiffReport) {
    let now = Utc::now().to_rfc3339();
    println!("# audit-harness Phase 2c — DB ↔ archive XDR re-parse\n");
    println!("**Timestamp:** {now}");
    println!("**Table:** `{}`", r.table);
    println!("**Sampled:** {}", r.sampled);
    println!("**Mismatched rows:** {}", r.mismatched_rows);
    println!("**Unreachable:** {}\n", r.unreachable);

    if r.mismatched_rows == 0 && r.unreachable == 0 {
        println!("✓ All sampled rows match the archive XDR field-for-field.");
        return;
    }
    if r.mismatched_rows > 0 {
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
