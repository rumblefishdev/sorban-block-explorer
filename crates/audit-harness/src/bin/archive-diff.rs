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
//! Two variants are wired today:
//!
//! - `--table ledgers` — fetch the LedgerCloseMeta from the public archive,
//!   re-derive header fields and the canonical hash, diff against
//!   `ledgers.hash / closed_at / protocol_version / transaction_count /
//!   base_fee`. Caught task 0181 at scale on the first run.
//! - `--table liquidity_pools` — reconstruct `LiquidityPoolParameters` XDR
//!   from DB-stored `(asset_a, asset_b, fee_bps)` rows, SHA-256 the
//!   canonical bytes per CAP-0038, and diff the resulting hash against
//!   `liquidity_pools.pool_id`. Closes the issuer-level acceptance criterion
//!   that Phase 1 I3 (`type, code` only) deliberately defers because
//!   surrogate-ID and base32-strkey order do not preserve canonical
//!   raw-byte order — see task 0179. No archive fetch — purely DB-local.
//!
//! Usage:
//!   DATABASE_URL=postgres://... cargo run -p audit-harness --bin archive-diff -- \
//!       --table ledgers --sample 50 --concurrency 4
//!   DATABASE_URL=postgres://... cargo run -p audit-harness --bin archive-diff -- \
//!       --table liquidity_pools --sample 100

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use clap::{Parser, ValueEnum};
use sha2::{Digest, Sha256};
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use stellar_xdr::curr::{
    AccountId, AlphaNum4, AlphaNum12, Asset, AssetCode4, AssetCode12, LedgerCloseMeta,
    LedgerCloseMetaBatch, Limits, LiquidityPoolConstantProductParameters, LiquidityPoolParameters,
    PublicKey, ReadXdr, Uint256, WriteXdr,
};
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
    /// `liquidity_pools` table — reconstruct `LiquidityPoolParameters`
    /// XDR from DB-stored `(asset_a, asset_b, fee_bps)`, SHA-256 the
    /// canonical bytes (CAP-0038), and diff the resulting hash against
    /// `liquidity_pools.pool_id`. Catches asset-pair canonical-order
    /// violations and any other bug that would corrupt `pool_id` —
    /// closes the issuer-level acceptance criterion that Phase 1 I3
    /// (`type, code` only) deliberately defers because surrogate-ID
    /// and base32-strkey order do not preserve canonical raw-byte order
    /// (see [task 0179](../../../lore/1-tasks/active/0179_BUG_lp-asset-canonical-order-violated.md)).
    /// No archive fetch — this variant is purely DB-local.
    LiquidityPools,
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
        Table::LiquidityPools => diff_liquidity_pools(&db, cli.sample).await?,
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
// Liquidity-pools diff (CAP-0038 protocol-hash verification)
// ---------------------------------------------------------------------------
//
// Reconstructs `LiquidityPoolParameters` XDR from DB rows and verifies that
// `SHA-256(canonical bytes) == pool_id`. No archive fetch — Stellar's protocol
// (CAP-0038) defines `pool_id` deterministically from the asset-pair tuple
// plus the fee, so the canonical bytes are reproducible from any source that
// preserves the pair faithfully. This is the issuer-level acceptance criterion
// that Phase 1 I3 (`type, code` only) deliberately defers because surrogate-ID
// and base32-strkey order do not preserve canonical raw-byte order — see
// task 0179 for the diagnosis.

#[derive(Debug)]
struct OurPool {
    pool_id_hex: String,
    asset_a_type: i16,
    asset_a_code: Option<String>,
    asset_a_issuer: Option<String>,
    asset_b_type: i16,
    asset_b_code: Option<String>,
    asset_b_issuer: Option<String>,
    fee_bps: i32,
}

/// DB asset-type SMALLINT mapping per `asset_type_name(smallint)` SQL helper
/// (ADR 0037 §"Enum label helper functions"):
///   0 = native, 1 = credit_alphanum4, 2 = credit_alphanum12, 3 = pool_share
///
/// `pool_share` is not a valid component for an LP and never appears in
/// `asset_*_type` of `liquidity_pools` rows (those are the underlying pair),
/// so this function rejects it.
fn build_asset(
    asset_type: i16,
    code: Option<&str>,
    issuer_strkey: Option<&str>,
) -> Result<Asset, Box<dyn std::error::Error + Send + Sync>> {
    match asset_type {
        0 => Ok(Asset::Native),
        1 | 2 => {
            let code = code.ok_or("alphanum asset requires non-NULL code")?;
            let issuer = issuer_strkey.ok_or("alphanum asset requires non-NULL issuer")?;
            let issuer_bytes = stellar_strkey::ed25519::PublicKey::from_string(issuer)
                .map_err(|e| format!("strkey decode failed for {issuer}: {e}"))?
                .0;
            let issuer_acc =
                AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(issuer_bytes)));
            // Code is stored NUL-stripped by the parser (see task 0179);
            // re-pad with NULs to the fixed XDR width (4 or 12).
            let code_bytes = code.as_bytes();
            if asset_type == 1 {
                if code_bytes.len() > 4 {
                    return Err(format!("alphanum4 code longer than 4 bytes: {code}").into());
                }
                let mut padded = [0u8; 4];
                padded[..code_bytes.len()].copy_from_slice(code_bytes);
                Ok(Asset::CreditAlphanum4(AlphaNum4 {
                    asset_code: AssetCode4(padded),
                    issuer: issuer_acc,
                }))
            } else {
                if code_bytes.len() > 12 {
                    return Err(format!("alphanum12 code longer than 12 bytes: {code}").into());
                }
                let mut padded = [0u8; 12];
                padded[..code_bytes.len()].copy_from_slice(code_bytes);
                Ok(Asset::CreditAlphanum12(AlphaNum12 {
                    asset_code: AssetCode12(padded),
                    issuer: issuer_acc,
                }))
            }
        }
        other => Err(format!(
            "unexpected asset type for LP component: {other} (expected 0, 1, or 2)"
        )
        .into()),
    }
}

/// Compute `SHA-256(LiquidityPoolParameters XDR)` per CAP-0038, returning the
/// hex-encoded 64-char hash for direct comparison to the DB `pool_id` column.
fn protocol_pool_id(asset_a: Asset, asset_b: Asset, fee_bps: i32) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let params = LiquidityPoolParameters::LiquidityPoolConstantProduct(
        LiquidityPoolConstantProductParameters {
            asset_a,
            asset_b,
            fee: fee_bps,
        },
    );
    let bytes = params.to_xdr(Limits::none())?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

async fn diff_liquidity_pools(
    db: &sqlx::PgPool,
    sample: usize,
) -> Result<DiffReport, Box<dyn std::error::Error + Send + Sync>> {
    // JOIN accounts to surface the natural strkey for each issuer surrogate
    // FK; native components have NULL on both code and issuer.
    let rows = sqlx::query(
        r#"
        SELECT
            encode(lp.pool_id, 'hex')             AS pool_id_hex,
            lp.asset_a_type                       AS asset_a_type,
            lp.asset_a_code                       AS asset_a_code,
            a_a.account_id                        AS asset_a_issuer,
            lp.asset_b_type                       AS asset_b_type,
            lp.asset_b_code                       AS asset_b_code,
            a_b.account_id                        AS asset_b_issuer,
            lp.fee_bps                            AS fee_bps
          FROM liquidity_pools lp
          LEFT JOIN accounts a_a ON a_a.id = lp.asset_a_issuer_id
          LEFT JOIN accounts a_b ON a_b.id = lp.asset_b_issuer_id
         ORDER BY random()
         LIMIT $1
        "#,
    )
    .bind(sample as i64)
    .fetch_all(db)
    .await?;

    let our: Vec<OurPool> = rows
        .iter()
        .map(|r| OurPool {
            pool_id_hex: r.get("pool_id_hex"),
            asset_a_type: r.get("asset_a_type"),
            asset_a_code: r.get("asset_a_code"),
            asset_a_issuer: r.get("asset_a_issuer"),
            asset_b_type: r.get("asset_b_type"),
            asset_b_code: r.get("asset_b_code"),
            asset_b_issuer: r.get("asset_b_issuer"),
            fee_bps: r.get("fee_bps"),
        })
        .collect();

    let mut report = empty_report("liquidity_pools");
    report.sampled = our.len();
    if our.is_empty() {
        return Ok(report);
    }

    for o in our {
        let key = format!("pool_id={}", o.pool_id_hex);
        let asset_a = match build_asset(
            o.asset_a_type,
            o.asset_a_code.as_deref(),
            o.asset_a_issuer.as_deref(),
        ) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(pool_id = %o.pool_id_hex, side = "a", error = %e, "asset reconstruction failed");
                report.unreachable += 1;
                continue;
            }
        };
        let asset_b = match build_asset(
            o.asset_b_type,
            o.asset_b_code.as_deref(),
            o.asset_b_issuer.as_deref(),
        ) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(pool_id = %o.pool_id_hex, side = "b", error = %e, "asset reconstruction failed");
                report.unreachable += 1;
                continue;
            }
        };
        let recomputed = match protocol_pool_id(asset_a, asset_b, o.fee_bps) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(pool_id = %o.pool_id_hex, error = %e, "XDR serialize failed");
                report.unreachable += 1;
                continue;
            }
        };
        if recomputed != o.pool_id_hex {
            report.mismatched_rows += 1;
            report.field_mismatches.push(FieldMismatch {
                key,
                field: "pool_id (SHA-256 LiquidityPoolParameters XDR)".into(),
                ours: o.pool_id_hex.clone(),
                theirs: recomputed,
            });
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
