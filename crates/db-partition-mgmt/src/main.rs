//! Partition management Lambda for the Soroban block explorer.
//!
//! Invoked as a CloudFormation custom resource (on deploy) and via
//! EventBridge schedule (monthly). Ensures partitions exist for all
//! partitioned tables — both historical (from Soroban activation) and
//! future (current month + 3).
//!
//! Publishes CloudWatch custom metrics for monitoring:
//! - `FuturePartitionCount` per time-based table
//! - `OperationsRangeUsagePercent` for the operations table

use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};
use chrono::{Datelike, NaiveDate, Utc};
use lambda_runtime::{Error, LambdaEvent, service_fn};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

const PHYSICAL_RESOURCE_ID: &str = "soroban-explorer-partition-mgmt";

/// Soroban activation date (Protocol 20, ledger 50,457,424).
const SOROBAN_START: (i32, u32) = (2024, 2);

/// How many months into the future to pre-create.
const FUTURE_MONTHS: u32 = 3;

/// Range size for operations partitions.
const OPERATIONS_RANGE_SIZE: i64 = 10_000_000;

/// Tables with monthly time-based partitions.
const TIME_PARTITIONED_TABLES: &[&str] = &[
    "soroban_invocations",
    "soroban_events",
    "liquidity_pool_snapshots",
];

// ───────────────────────── Handler ─────────────────────────

async fn handler(event: LambdaEvent<Value>) -> Result<Value, Error> {
    let (payload, _context) = event.into_parts();

    // CloudFormation custom resource sends RequestType; EventBridge does not.
    // Default to "Create" so scheduled invocations run the partition logic.
    let request_type = payload["RequestType"].as_str().unwrap_or("Create");
    tracing::info!(request_type, "partition-mgmt handler invoked");

    if request_type == "Delete" {
        tracing::info!("delete event — no-op for partition management");
        return Ok(json!({
            "PhysicalResourceId": PHYSICAL_RESOURCE_ID,
            "Data": { "Message": "No action on delete" }
        }));
    }

    let secret_arn = std::env::var("SECRET_ARN").map_err(|_| "SECRET_ARN not set")?;
    let rds_endpoint =
        std::env::var("RDS_PROXY_ENDPOINT").map_err(|_| "RDS_PROXY_ENDPOINT not set")?;
    let env_name = std::env::var("ENV_NAME").unwrap_or_else(|_| "unknown".into());

    let database_url = db::secrets::resolve_database_url(&secret_arn, &rds_endpoint).await?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await?;

    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let cw_client = aws_sdk_cloudwatch::Client::new(&aws_config);

    // ── Time-based partitions ──
    let now = Utc::now().naive_utc().date();
    let mut total_created = 0u32;
    let mut metrics = Vec::new();

    for table in TIME_PARTITIONED_TABLES {
        let created = ensure_time_partitions(&pool, table, now).await?;
        total_created += created;

        let future_count = count_future_partitions(&pool, table, now).await?;
        tracing::info!(table, created, future_count, "time partitions ensured");

        metrics.push(
            MetricDatum::builder()
                .metric_name("FuturePartitionCount")
                .dimensions(Dimension::builder().name("Table").value(*table).build())
                .value(future_count as f64)
                .unit(StandardUnit::Count)
                .build(),
        );
    }

    // ── Operations (transaction_id range) ──
    let ops_result = ensure_operations_partitions(&pool).await?;
    total_created += ops_result.partitions_created;
    tracing::info!(
        created = ops_result.partitions_created,
        usage_pct = ops_result.usage_percent,
        max_id = ops_result.max_transaction_id,
        "operations partitions ensured"
    );

    metrics.push(
        MetricDatum::builder()
            .metric_name("OperationsRangeUsagePercent")
            .value(ops_result.usage_percent)
            .unit(StandardUnit::Percent)
            .build(),
    );

    // ── Publish CloudWatch metrics ──
    if !metrics.is_empty() {
        let namespace = format!("SorobanExplorer/{env_name}/Partitions");
        cw_client
            .put_metric_data()
            .namespace(&namespace)
            .set_metric_data(Some(metrics))
            .send()
            .await
            .map_err(|e| format!("CloudWatch PutMetricData failed: {e}"))?;
        tracing::info!(namespace, "metrics published");
    }

    pool.close().await;

    Ok(json!({
        "PhysicalResourceId": PHYSICAL_RESOURCE_ID,
        "Data": {
            "Message": format!("{total_created} partitions created"),
            "TotalCreated": total_created
        }
    }))
}

// ──────────────── Pure decision functions (testable) ───────────────

/// Returns partition names that need to be created for a time-based table.
/// Covers from Soroban activation (2024-02) to `today + FUTURE_MONTHS`.
fn months_to_create(
    table: &str,
    existing: &[String],
    today: NaiveDate,
) -> Vec<(String, NaiveDate)> {
    let start =
        NaiveDate::from_ymd_opt(SOROBAN_START.0, SOROBAN_START.1, 1).expect("valid SOROBAN_START");
    let end = add_months(today, FUTURE_MONTHS);

    let mut missing = Vec::new();
    let mut cursor = start;

    while cursor <= end {
        let name = format!("{}_y{}m{:02}", table, cursor.year(), cursor.month());
        if !existing.contains(&name) {
            missing.push((name, cursor));
        }
        cursor = add_months(cursor, 1);
    }

    missing
}

/// Computes operations range usage and which new partitions to create.
/// Returns (usage_percent, vec of (name, range_start, range_end) to create).
fn operations_ranges_to_create(
    existing: &[String],
    max_transaction_id: i64,
) -> (f64, Vec<(String, i64, i64)>) {
    let highest_range_end = existing
        .iter()
        .filter_map(|name| parse_operations_range_end(name))
        .max()
        .unwrap_or(OPERATIONS_RANGE_SIZE);

    let usage_percent = if highest_range_end > 0 {
        (max_transaction_id as f64 / highest_range_end as f64) * 100.0
    } else {
        0.0
    };

    let mut to_create = Vec::new();

    if usage_percent > 80.0 || max_transaction_id >= highest_range_end {
        let mut range_start = highest_range_end;
        while range_start <= max_transaction_id + OPERATIONS_RANGE_SIZE {
            let range_end = range_start + OPERATIONS_RANGE_SIZE;
            let n = range_start / OPERATIONS_RANGE_SIZE;
            let name = format!("operations_p{n}");
            if !existing.contains(&name) {
                to_create.push((name, range_start, range_end));
            }
            range_start = range_end;
        }
    }

    (usage_percent, to_create)
}

// ───────────────── Time-based partition logic ──────────────────

/// Ensures monthly partitions exist from Soroban activation to now + FUTURE_MONTHS.
async fn ensure_time_partitions(
    pool: &PgPool,
    table: &str,
    today: NaiveDate,
) -> Result<u32, Error> {
    let existing = get_existing_partitions(pool, table).await?;
    let missing = months_to_create(table, &existing, today);

    let mut created = 0u32;
    for (name, month_start) in &missing {
        let from = month_start.format("%Y-%m-%d 00:00:00+00");
        let to = add_months(*month_start, 1).format("%Y-%m-%d 00:00:00+00");

        let create_ddl = format!(
            "CREATE TABLE {name} PARTITION OF {table} \
             FOR VALUES FROM ('{from}') TO ('{to}')"
        );

        match sqlx::query(&create_ddl).execute(pool).await {
            Ok(_) => {
                tracing::info!(partition = %name, "created");
                created += 1;
            }
            // 42P07 = duplicate_table — table exists but may be detached; reattach it.
            Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("42P07") => {
                let attach_ddl = format!(
                    "ALTER TABLE {table} ATTACH PARTITION {name} \
                     FOR VALUES FROM ('{from}') TO ('{to}')"
                );
                sqlx::query(&attach_ddl).execute(pool).await?;
                tracing::info!(partition = %name, "reattached");
                created += 1;
            }
            Err(err) => return Err(err.into()),
        }
    }

    Ok(created)
}

/// Counts partitions that cover months strictly after today.
async fn count_future_partitions(
    pool: &PgPool,
    table: &str,
    today: NaiveDate,
) -> Result<u32, Error> {
    let current_month_start =
        NaiveDate::from_ymd_opt(today.year(), today.month(), 1).ok_or("invalid date")?;
    let next_month = add_months(current_month_start, 1);

    let existing = get_existing_partitions(pool, table).await?;
    let future = existing
        .iter()
        .filter(|name| {
            parse_partition_month(name)
                .map(|d| d >= next_month)
                .unwrap_or(false)
        })
        .count();

    Ok(future as u32)
}

/// Queries pg_inherits to list child partition names (excluding _default).
async fn get_existing_partitions(pool: &PgPool, parent_table: &str) -> Result<Vec<String>, Error> {
    let rows = sqlx::query(
        "SELECT c.relname::text \
         FROM pg_inherits i \
         JOIN pg_class c ON c.oid = i.inhrelid \
         JOIN pg_class p ON p.oid = i.inhparent \
         WHERE p.relname = $1 \
           AND c.relname NOT LIKE '%_default' \
         ORDER BY c.relname",
    )
    .bind(parent_table)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(|r| r.get::<String, _>(0)).collect())
}

/// Parses `table_y2026m04` → NaiveDate(2026, 4, 1).
fn parse_partition_month(name: &str) -> Option<NaiveDate> {
    let y_pos = name.rfind("_y")?;
    let suffix = &name[y_pos + 2..];
    let m_pos = suffix.find('m')?;
    let year: i32 = suffix[..m_pos].parse().ok()?;
    let month: u32 = suffix[m_pos + 1..].parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, 1)
}

// ──────────────── Operations partition logic ───────────────────

struct OperationsResult {
    partitions_created: u32,
    usage_percent: f64,
    max_transaction_id: i64,
}

/// Ensures the operations table has a range partition covering the current
/// max transaction_id with headroom. Creates next range if >80% consumed.
async fn ensure_operations_partitions(pool: &PgPool) -> Result<OperationsResult, Error> {
    let max_id: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM transactions")
        .fetch_one(pool)
        .await?;

    let existing = get_existing_partitions(pool, "operations").await?;
    let (usage_percent, to_create) = operations_ranges_to_create(&existing, max_id);

    let mut created = 0u32;
    for (name, range_start, range_end) in &to_create {
        let create_ddl = format!(
            "CREATE TABLE {name} PARTITION OF operations \
             FOR VALUES FROM ({range_start}) TO ({range_end})"
        );

        match sqlx::query(&create_ddl).execute(pool).await {
            Ok(_) => {
                tracing::info!(partition = %name, range_start, range_end, "created");
                created += 1;
            }
            Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("42P07") => {
                let attach_ddl = format!(
                    "ALTER TABLE operations ATTACH PARTITION {name} \
                     FOR VALUES FROM ({range_start}) TO ({range_end})"
                );
                sqlx::query(&attach_ddl).execute(pool).await?;
                tracing::info!(partition = %name, range_start, range_end, "reattached");
                created += 1;
            }
            Err(err) => return Err(err.into()),
        }
    }

    Ok(OperationsResult {
        partitions_created: created,
        usage_percent,
        max_transaction_id: max_id,
    })
}

/// Parses `operations_p1` → upper bound 20_000_000 (p1 covers 10M..20M).
fn parse_operations_range_end(name: &str) -> Option<i64> {
    let suffix = name.strip_prefix("operations_p")?;
    let n: i64 = suffix.parse().ok()?;
    Some((n + 1) * OPERATIONS_RANGE_SIZE)
}

// ────────────────────────── Helpers ────────────────────────────

/// Adds N months to a NaiveDate (clamped to 1st of month).
fn add_months(date: NaiveDate, months: u32) -> NaiveDate {
    let total_months = date.year() * 12 + date.month() as i32 - 1 + months as i32;
    let year = total_months / 12;
    let month = (total_months % 12) + 1;
    NaiveDate::from_ymd_opt(year, month as u32, 1).unwrap_or(date)
}

// ──────────────────────────── Main ─────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    lambda_runtime::run(service_fn(handler)).await
}

// ────────────────────────── Tests ──────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Parsing tests ──

    #[test]
    fn parse_partition_month_valid() {
        assert_eq!(
            parse_partition_month("soroban_events_y2026m04"),
            Some(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap())
        );
        assert_eq!(
            parse_partition_month("liquidity_pool_snapshots_y2024m12"),
            Some(NaiveDate::from_ymd_opt(2024, 12, 1).unwrap())
        );
    }

    #[test]
    fn parse_partition_month_invalid() {
        assert_eq!(parse_partition_month("soroban_events_default"), None);
        assert_eq!(parse_partition_month("random_name"), None);
    }

    #[test]
    fn parse_operations_range_end_valid() {
        assert_eq!(
            parse_operations_range_end("operations_p0"),
            Some(10_000_000)
        );
        assert_eq!(
            parse_operations_range_end("operations_p1"),
            Some(20_000_000)
        );
        assert_eq!(
            parse_operations_range_end("operations_p5"),
            Some(60_000_000)
        );
    }

    #[test]
    fn parse_operations_range_end_invalid() {
        assert_eq!(parse_operations_range_end("operations_default"), None);
        assert_eq!(parse_operations_range_end("not_operations"), None);
    }

    // ── Date arithmetic tests ──

    #[test]
    fn add_months_basic() {
        let jan = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert_eq!(
            add_months(jan, 1),
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()
        );
        assert_eq!(
            add_months(jan, 12),
            NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        );
    }

    #[test]
    fn add_months_year_boundary() {
        let nov = NaiveDate::from_ymd_opt(2025, 11, 1).unwrap();
        assert_eq!(
            add_months(nov, 3),
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()
        );
    }

    // ── Decision logic tests: months_to_create ──

    #[test]
    fn months_to_create_fills_gap() {
        // Existing: only Apr 2026. Today: Apr 2026.
        // Should create: 2024-02 through 2026-07 minus 2026-04 = 26 months
        let existing = vec!["soroban_events_y2026m04".to_string()];
        let today = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
        let missing = months_to_create("soroban_events", &existing, today);

        // Should not include the existing partition
        assert!(!missing.iter().any(|(n, _)| n == "soroban_events_y2026m04"));
        // Should include Soroban start
        assert!(missing.iter().any(|(n, _)| n == "soroban_events_y2024m02"));
        // Should include 3 months ahead (Jul 2026)
        assert!(missing.iter().any(|(n, _)| n == "soroban_events_y2026m07"));
        // Should NOT include Aug 2026 (4 months ahead)
        assert!(!missing.iter().any(|(n, _)| n == "soroban_events_y2026m08"));
    }

    #[test]
    fn months_to_create_all_exist() {
        // today=2024-03-01, future=3 → need 2024-02 through 2024-06
        let today = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
        let existing: Vec<String> = (2..=6)
            .map(|m| format!("soroban_events_y2024m{m:02}"))
            .collect();
        let missing = months_to_create("soroban_events", &existing, today);
        assert!(missing.is_empty());
    }

    #[test]
    fn months_to_create_upper_bound() {
        // today=2024-03-01, existing covers 02-05 → missing only 2024-06
        let today = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
        let existing: Vec<String> = (2..=5)
            .map(|m| format!("soroban_events_y2024m{m:02}"))
            .collect();
        let missing = months_to_create("soroban_events", &existing, today);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, "soroban_events_y2024m06");
    }

    // ── Decision logic tests: operations_ranges_to_create ──

    #[test]
    fn operations_no_expansion_under_80pct() {
        let existing = vec!["operations_p0".to_string()];
        let (usage, to_create) = operations_ranges_to_create(&existing, 5_000_000);
        assert_eq!(usage, 50.0);
        assert!(to_create.is_empty());
    }

    #[test]
    fn operations_expands_at_80pct() {
        let existing = vec!["operations_p0".to_string()];
        let (usage, to_create) = operations_ranges_to_create(&existing, 8_500_000);
        assert!(usage > 80.0);
        assert!(!to_create.is_empty());
        assert_eq!(to_create[0].0, "operations_p1");
        assert_eq!(to_create[0].1, 10_000_000);
        assert_eq!(to_create[0].2, 20_000_000);
    }

    #[test]
    fn operations_expands_past_current_range() {
        // max_id exceeds current range — should create enough partitions
        let existing = vec!["operations_p0".to_string()];
        let (_, to_create) = operations_ranges_to_create(&existing, 15_000_000);
        // Need p1 (10M-20M) and p2 (20M-30M) for headroom
        assert!(to_create.len() >= 2);
    }

    #[test]
    fn operations_empty_db() {
        let existing = vec!["operations_p0".to_string()];
        let (usage, to_create) = operations_ranges_to_create(&existing, 0);
        assert_eq!(usage, 0.0);
        assert!(to_create.is_empty());
    }
}
