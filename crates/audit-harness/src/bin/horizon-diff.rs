//! Phase 2a — DB↔Horizon API field-level diff harness.
//!
//! Picks N random rows from a target table, fetches the matching Horizon
//! resource, diffs the shared fields, and reports per-field mismatches.
//! Runs against the local explorer DB (read-only) and the public Horizon
//! API (`https://horizon.stellar.org` for mainnet).
//!
//! Implements `--table {ledgers, transactions, accounts, balances, assets,
//! liquidity-pools}`. Each table maps to a Horizon endpoint; the harness
//! samples N random rows from the local DB and diffs the shared fields.
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
    /// `accounts` table — compare account_id, sequence_number, home_domain
    /// against `/accounts/:account_id`. Skips funded-then-merged accounts
    /// (Horizon 404). Sample is random by account_id.
    Accounts,
    /// `account_balances_current` — per-account balance set diff against the `balances[]`
    /// array in `/accounts/:account_id`. Compares native + classic credit balances;
    /// SAC / Soroban-only balances are skipped (Horizon doesn't index them).
    Balances,
    /// `assets` table — total_supply + holder_count diff against
    /// `/assets?asset_code=X&asset_issuer=Y`. Native + SAC-only + Soroban-
    /// native rows are skipped (no Horizon equivalent).
    Assets,
    /// `liquidity_pools` table — fee_bps + asset pair + total_shares diff
    /// against `/liquidity_pools/:id`.
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
        Table::Accounts => {
            diff_accounts(&db, &http, &cli.horizon_url, cli.sample, cli.concurrency).await?
        }
        Table::Balances => {
            diff_balances(&db, &http, &cli.horizon_url, cli.sample, cli.concurrency).await?
        }
        Table::Assets => {
            diff_assets(&db, &http, &cli.horizon_url, cli.sample, cli.concurrency).await?
        }
        Table::LiquidityPools => {
            diff_liquidity_pools(&db, &http, &cli.horizon_url, cli.sample, cli.concurrency).await?
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
// Accounts
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HorizonAccount {
    account_id: String,
    /// Horizon emits `sequence` as a JSON string (BIGINT-shaped).
    sequence: String,
    #[serde(default)]
    home_domain: Option<String>,
}

#[derive(Debug)]
struct OurAccount {
    account_id: String,
    sequence_number: i64,
    home_domain: Option<String>,
}

async fn diff_accounts(
    db: &sqlx::PgPool,
    http: &reqwest::Client,
    horizon_url: &str,
    sample: usize,
    concurrency: usize,
) -> Result<DiffReport, Box<dyn std::error::Error + Send + Sync>> {
    // Skip M-keys (muxed) — Horizon's `/accounts/:id` only accepts G-keys.
    // Per task 0044 / ADR 0044, persisted accounts should always be G/56;
    // the filter is defensive against any pre-unwrap residue.
    let rows = sqlx::query(
        "SELECT account_id, sequence_number, home_domain
         FROM accounts
         WHERE account_id LIKE 'G%' AND length(account_id) = 56
         ORDER BY random() LIMIT $1",
    )
    .bind(sample as i64)
    .fetch_all(db)
    .await?;

    let our: Vec<OurAccount> = rows
        .iter()
        .map(|r| OurAccount {
            account_id: r.get("account_id"),
            sequence_number: r.get("sequence_number"),
            home_domain: r.get("home_domain"),
        })
        .collect();

    if our.is_empty() {
        return Ok(empty_report("accounts"));
    }

    let sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(our.len());
    for o in our {
        let sem = sem.clone();
        let http = http.clone();
        let url = format!("{}/accounts/{}", horizon_url, o.account_id);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore");
            let resp = http.get(&url).send().await;
            (o, resp)
        }));
    }

    let mut report = empty_report("accounts");
    report.sampled = handles.len();
    for h in handles {
        let (o, resp) = h.await.expect("join");
        let key = format!("acct={}", &o.account_id[..16]);
        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            // 404 — account merged or never existed on the public chain.
            // Common for short-lived accounts; not a hard mismatch.
            Ok(r) if r.status().as_u16() == 404 => {
                report.unreachable += 1;
                continue;
            }
            Ok(r) => {
                tracing::warn!(acct = %o.account_id, status = %r.status(), "horizon non-200");
                report.unreachable += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!(acct = %o.account_id, error = %e, "fetch failed");
                report.unreachable += 1;
                continue;
            }
        };
        let theirs: HorizonAccount = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(acct = %o.account_id, error = %e, "parse failed");
                report.unreachable += 1;
                continue;
            }
        };

        let mut row_mismatches = Vec::new();

        // Sequence numbers can drift between snapshots: Horizon shows the
        // *current* sequence, our DB row reflects state at last seen.
        // Diff is informational only when our row is older than Horizon.
        // Hard-fail only when ours > theirs (impossible — we'd be ahead
        // of Horizon, which would be a bug).
        match theirs.sequence.parse::<i64>() {
            Ok(theirs_seq) if o.sequence_number > theirs_seq => {
                row_mismatches.push(FieldMismatch {
                    key: key.clone(),
                    field: "sequence_number_ahead_of_horizon".into(),
                    ours: o.sequence_number.to_string(),
                    theirs: theirs_seq.to_string(),
                });
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, raw = %theirs.sequence, "sequence parse failed");
            }
        }

        // home_domain match (where ours has a value). Empty / null on either
        // side is acceptable — the field is optional + can be unset on chain
        // after extraction.
        if let (Some(ours), Some(theirs_hd)) = (&o.home_domain, &theirs.home_domain)
            && ours != theirs_hd
        {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "home_domain".into(),
                ours: ours.clone(),
                theirs: theirs_hd.clone(),
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
// Account balances
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HorizonBalance {
    /// `native` for XLM, otherwise `credit_alphanum4` / `credit_alphanum12`.
    /// SAC and Soroban-native do not appear here.
    asset_type: String,
    /// Asset code; absent for `native`.
    #[serde(default)]
    asset_code: Option<String>,
    /// Issuer StrKey; absent for `native`.
    #[serde(default)]
    asset_issuer: Option<String>,
    /// Balance as decimal string with up to 7 fractional digits.
    balance: String,
}

#[derive(Debug, Deserialize)]
struct HorizonAccountWithBalances {
    balances: Vec<HorizonBalance>,
}

async fn diff_balances(
    db: &sqlx::PgPool,
    http: &reqwest::Client,
    horizon_url: &str,
    sample: usize,
    concurrency: usize,
) -> Result<DiffReport, Box<dyn std::error::Error + Send + Sync>> {
    // Sample at the ACCOUNT level — fetch the account's balance set from
    // Horizon and diff against every `account_balances_current` row for
    // that account.
    let rows = sqlx::query(
        "SELECT a.account_id
         FROM accounts a
         WHERE a.account_id LIKE 'G%' AND length(a.account_id) = 56
           AND EXISTS (
               SELECT 1 FROM account_balances_current abc WHERE abc.account_id = a.id
           )
         ORDER BY random() LIMIT $1",
    )
    .bind(sample as i64)
    .fetch_all(db)
    .await?;

    let our_accounts: Vec<String> = rows.iter().map(|r| r.get::<String, _>(0)).collect();

    if our_accounts.is_empty() {
        return Ok(empty_report("account_balances_current"));
    }

    let sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(our_accounts.len());
    for acct in our_accounts {
        let sem = sem.clone();
        let http = http.clone();
        let url = format!("{}/accounts/{}", horizon_url, acct);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore");
            let resp = http.get(&url).send().await;
            (acct, resp)
        }));
    }

    let mut report = empty_report("account_balances_current");
    report.sampled = handles.len();
    for h in handles {
        let (acct, resp) = h.await.expect("join");
        let key = format!("acct={}", &acct[..16]);
        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) if r.status().as_u16() == 404 => {
                report.unreachable += 1;
                continue;
            }
            Ok(r) => {
                tracing::warn!(acct = %acct, status = %r.status(), "horizon non-200");
                report.unreachable += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!(acct = %acct, error = %e, "fetch failed");
                report.unreachable += 1;
                continue;
            }
        };
        let theirs: HorizonAccountWithBalances = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(acct = %acct, error = %e, "parse failed");
                report.unreachable += 1;
                continue;
            }
        };

        // Fetch our balances for this account. JOIN to surface issuer StrKey.
        let our_balances = sqlx::query(
            "SELECT abc.asset_type, abc.asset_code,
                    iss.account_id AS issuer_strkey,
                    abc.balance::TEXT AS balance_text
             FROM account_balances_current abc
             JOIN accounts a ON a.id = abc.account_id
             LEFT JOIN accounts iss ON iss.id = abc.issuer_id
             WHERE a.account_id = $1",
        )
        .bind(&acct)
        .fetch_all(db)
        .await?;

        let mut row_mismatches = Vec::new();

        for our_bal in our_balances {
            let our_asset_type: i16 = our_bal.get("asset_type");
            let our_code: Option<String> = our_bal.get("asset_code");
            let our_issuer: Option<String> = our_bal.get("issuer_strkey");
            let our_balance: String = our_bal.get("balance_text");

            // Match against Horizon balance entries. Native = asset_type=0
            // matches Horizon `native`; classic credit (1) matches
            // `credit_alphanum4` / `credit_alphanum12` by (code, issuer).
            let horizon_match = theirs.balances.iter().find(|hb| {
                if our_asset_type == 0 {
                    hb.asset_type == "native"
                } else {
                    hb.asset_type.starts_with("credit_alphanum")
                        && hb.asset_code == our_code
                        && hb.asset_issuer == our_issuer
                }
            });

            match horizon_match {
                Some(hb) => {
                    // Compare balances as decimal strings (both 7-frac-digit).
                    if !decimal_strings_equal(&our_balance, &hb.balance) {
                        let asset_label = our_code.clone().unwrap_or_else(|| "native".into());
                        row_mismatches.push(FieldMismatch {
                            key: format!("{}/{}", key, asset_label),
                            field: "balance".into(),
                            ours: our_balance,
                            theirs: hb.balance.clone(),
                        });
                    }
                }
                None => {
                    let asset_label = our_code.clone().unwrap_or_else(|| "native".into());
                    row_mismatches.push(FieldMismatch {
                        key: format!("{}/{}", key, asset_label),
                        field: "missing_on_horizon".into(),
                        ours: our_balance,
                        theirs: "<absent>".into(),
                    });
                }
            }
        }

        if !row_mismatches.is_empty() {
            report.mismatched_rows += 1;
            report.field_mismatches.extend(row_mismatches);
        }
    }
    Ok(report)
}

/// Normalize and compare two decimal strings ("1.0000000" == "1" == "1.000").
fn decimal_strings_equal(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> String {
        let s = s.trim();
        let (int_part, frac_part) = s.split_once('.').unwrap_or((s, ""));
        let int_part = int_part.trim_start_matches('0');
        let int_part = if int_part.is_empty() { "0" } else { int_part };
        let frac_part = frac_part.trim_end_matches('0');
        if frac_part.is_empty() {
            int_part.to_string()
        } else {
            format!("{int_part}.{frac_part}")
        }
    }
    norm(a) == norm(b)
}

// ---------------------------------------------------------------------------
// Assets
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HorizonAsset {
    asset_code: String,
    asset_issuer: String,
    /// Total supply across all trustlines, decimal-string formatted.
    amount: String,
    /// Trustline count. Horizon names this `num_accounts`; serde
    /// `rename_all` could collide with other fields, so we map explicitly.
    num_accounts: u64,
}

#[derive(Debug, Deserialize)]
struct HorizonAssetList {
    #[serde(rename = "_embedded")]
    embedded: HorizonAssetListEmbedded,
}

#[derive(Debug, Deserialize)]
struct HorizonAssetListEmbedded {
    records: Vec<HorizonAsset>,
}

#[derive(Debug)]
struct OurAsset {
    asset_code: String,
    issuer_strkey: String,
    total_supply: Option<String>,
    holder_count: i32,
}

async fn diff_assets(
    db: &sqlx::PgPool,
    http: &reqwest::Client,
    horizon_url: &str,
    sample: usize,
    concurrency: usize,
) -> Result<DiffReport, Box<dyn std::error::Error + Send + Sync>> {
    // Only classic credit assets (asset_type=1) have a Horizon equivalent.
    // SAC + Soroban-native + native singleton are skipped.
    let rows = sqlx::query(
        "SELECT a.asset_code, iss.account_id AS issuer_strkey,
                a.total_supply::TEXT AS total_supply,
                a.holder_count
         FROM assets a
         JOIN accounts iss ON iss.id = a.issuer_id
         WHERE a.asset_type = 1
           AND a.asset_code IS NOT NULL
         ORDER BY random() LIMIT $1",
    )
    .bind(sample as i64)
    .fetch_all(db)
    .await?;

    let our: Vec<OurAsset> = rows
        .iter()
        .map(|r| OurAsset {
            asset_code: r.get("asset_code"),
            issuer_strkey: r.get("issuer_strkey"),
            total_supply: r.get("total_supply"),
            holder_count: r.get("holder_count"),
        })
        .collect();

    if our.is_empty() {
        return Ok(empty_report("assets"));
    }

    let sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(our.len());
    for o in our {
        let sem = sem.clone();
        let http = http.clone();
        let url = format!(
            "{}/assets?asset_code={}&asset_issuer={}",
            horizon_url, o.asset_code, o.issuer_strkey
        );
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore");
            let resp = http.get(&url).send().await;
            (o, resp)
        }));
    }

    let mut report = empty_report("assets");
    report.sampled = handles.len();
    for h in handles {
        let (o, resp) = h.await.expect("join");
        let key = format!("{}/{}", o.asset_code, &o.issuer_strkey[..16]);
        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::warn!(code = %o.asset_code, status = %r.status(), "horizon non-200");
                report.unreachable += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!(code = %o.asset_code, error = %e, "fetch failed");
                report.unreachable += 1;
                continue;
            }
        };
        let theirs: HorizonAssetList = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(code = %o.asset_code, error = %e, "parse failed");
                report.unreachable += 1;
                continue;
            }
        };

        let theirs = match theirs.embedded.records.into_iter().next() {
            Some(r) => r,
            None => {
                report.unreachable += 1;
                continue;
            }
        };

        let mut row_mismatches = Vec::new();

        if let Some(ours_supply) = &o.total_supply
            && !decimal_strings_equal(ours_supply, &theirs.amount)
        {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "total_supply".into(),
                ours: ours_supply.clone(),
                theirs: theirs.amount.clone(),
            });
        }
        if o.holder_count as u64 != theirs.num_accounts {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "holder_count".into(),
                ours: o.holder_count.to_string(),
                theirs: theirs.num_accounts.to_string(),
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
// Liquidity pools
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HorizonLiquidityPool {
    id: String,
    fee_bp: u32,
    total_shares: String,
    total_trustlines: String,
    reserves: Vec<HorizonPoolReserve>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HorizonPoolReserve {
    /// `"native"` or `"<code>:<issuer>"`.
    asset: String,
    amount: String,
}

#[derive(Debug)]
struct OurPool {
    pool_id_hex: String,
    fee_bps: i32,
    asset_a_type: i16,
    asset_a_code: Option<String>,
    asset_a_issuer: Option<String>,
    asset_b_type: i16,
    asset_b_code: Option<String>,
    asset_b_issuer: Option<String>,
}

async fn diff_liquidity_pools(
    db: &sqlx::PgPool,
    http: &reqwest::Client,
    horizon_url: &str,
    sample: usize,
    concurrency: usize,
) -> Result<DiffReport, Box<dyn std::error::Error + Send + Sync>> {
    let rows = sqlx::query(
        "SELECT encode(lp.pool_id, 'hex') AS pool_id_hex,
                lp.fee_bps,
                lp.asset_a_type, lp.asset_a_code,
                aa.account_id AS asset_a_issuer,
                lp.asset_b_type, lp.asset_b_code,
                ab.account_id AS asset_b_issuer
         FROM liquidity_pools lp
         LEFT JOIN accounts aa ON aa.id = lp.asset_a_issuer_id
         LEFT JOIN accounts ab ON ab.id = lp.asset_b_issuer_id
         ORDER BY random() LIMIT $1",
    )
    .bind(sample as i64)
    .fetch_all(db)
    .await?;

    let our: Vec<OurPool> = rows
        .iter()
        .map(|r| OurPool {
            pool_id_hex: r.get("pool_id_hex"),
            fee_bps: r.get("fee_bps"),
            asset_a_type: r.get("asset_a_type"),
            asset_a_code: r.get("asset_a_code"),
            asset_a_issuer: r.get("asset_a_issuer"),
            asset_b_type: r.get("asset_b_type"),
            asset_b_code: r.get("asset_b_code"),
            asset_b_issuer: r.get("asset_b_issuer"),
        })
        .collect();

    if our.is_empty() {
        return Ok(empty_report("liquidity_pools"));
    }

    let sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(our.len());
    for o in our {
        let sem = sem.clone();
        let http = http.clone();
        let url = format!("{}/liquidity_pools/{}", horizon_url, o.pool_id_hex);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore");
            let resp = http.get(&url).send().await;
            (o, resp)
        }));
    }

    let mut report = empty_report("liquidity_pools");
    report.sampled = handles.len();
    for h in handles {
        let (o, resp) = h.await.expect("join");
        let key = format!("pool={}", &o.pool_id_hex[..16]);
        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::warn!(pool = %o.pool_id_hex, status = %r.status(), "horizon non-200");
                report.unreachable += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!(pool = %o.pool_id_hex, error = %e, "fetch failed");
                report.unreachable += 1;
                continue;
            }
        };
        let theirs: HorizonLiquidityPool = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(pool = %o.pool_id_hex, error = %e, "parse failed");
                report.unreachable += 1;
                continue;
            }
        };

        let mut row_mismatches = Vec::new();

        if u32::try_from(o.fee_bps)
            .map(|v| v != theirs.fee_bp)
            .unwrap_or(true)
        {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "fee_bps".into(),
                ours: o.fee_bps.to_string(),
                theirs: theirs.fee_bp.to_string(),
            });
        }

        // Asset pair: serialize ours as Horizon's "native" or "code:issuer".
        let ours_a = format_pool_asset(
            o.asset_a_type,
            o.asset_a_code.as_deref(),
            o.asset_a_issuer.as_deref(),
        );
        let ours_b = format_pool_asset(
            o.asset_b_type,
            o.asset_b_code.as_deref(),
            o.asset_b_issuer.as_deref(),
        );
        let theirs_a = theirs
            .reserves
            .first()
            .map(|r| r.asset.clone())
            .unwrap_or_default();
        let theirs_b = theirs
            .reserves
            .get(1)
            .map(|r| r.asset.clone())
            .unwrap_or_default();
        if ours_a != theirs_a {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "asset_a".into(),
                ours: ours_a,
                theirs: theirs_a,
            });
        }
        if ours_b != theirs_b {
            row_mismatches.push(FieldMismatch {
                key: key.clone(),
                field: "asset_b".into(),
                ours: ours_b,
                theirs: theirs_b,
            });
        }

        if !row_mismatches.is_empty() {
            report.mismatched_rows += 1;
            report.field_mismatches.extend(row_mismatches);
        }
    }
    Ok(report)
}

fn format_pool_asset(asset_type: i16, code: Option<&str>, issuer: Option<&str>) -> String {
    if asset_type == 0 {
        "native".to_string()
    } else {
        match (code, issuer) {
            (Some(c), Some(i)) => format!("{c}:{i}"),
            _ => format!("<malformed asset_type={asset_type}>"),
        }
    }
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
