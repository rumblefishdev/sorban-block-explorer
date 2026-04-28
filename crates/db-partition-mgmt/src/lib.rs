//! Partition management logic for the Soroban block explorer.
//!
//! All seven partitioned tables (`transactions`, `operations`,
//! `transaction_participants`, `soroban_invocations_appearances`,
//! `soroban_events_appearances`, `nft_ownership`, `liquidity_pool_snapshots`)
//! partition by `RANGE (created_at)` per ADR 0027.
//! This crate ensures monthly partitions exist from Soroban activation to
//! `today + FUTURE_MONTHS`.
//!
//! Publishes CloudWatch custom metric `FuturePartitionCount` per table.

use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};
use chrono::{Datelike, NaiveDate, Utc};
use lambda_runtime::{Error, LambdaEvent};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

pub const PHYSICAL_RESOURCE_ID: &str = "soroban-explorer-partition-mgmt";

/// Soroban activation date (Protocol 20, ledger 50,457,424).
pub const SOROBAN_START: (i32, u32) = (2024, 2);

/// How many months into the future to pre-create.
pub const FUTURE_MONTHS: u32 = 3;

/// Tables partitioned by RANGE (created_at) per ADR 0027.
pub const TIME_PARTITIONED_TABLES: &[&str] = &[
    "transactions",
    "operations_appearances",
    "transaction_participants",
    "soroban_invocations_appearances",
    "soroban_events_appearances",
    "nft_ownership",
    "liquidity_pool_snapshots",
];

// ───────────────────────── Handler ─────────────────────────

pub async fn handler(event: LambdaEvent<Value>) -> Result<Value, Error> {
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

    let today = Utc::now().naive_utc().date();
    let mut total_created = 0u32;
    let mut metrics = Vec::new();

    for table in TIME_PARTITIONED_TABLES {
        // Same primitive ordering as the CLI's `ensure_all_partitions`:
        // `_default` first so any out-of-range row has somewhere to land,
        // then the monthly children. Per-table loop is preserved here so
        // CloudWatch metrics still publish per-table dimensions.
        ensure_default_partition(&pool, table).await?;
        let created = ensure_time_partitions(&pool, table, today).await?;
        total_created += created;

        let future_count = count_future_partitions(&pool, table, today).await?;
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
pub fn months_to_create(
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

// ───────────────── Time-based partition logic ──────────────────

/// Ensures monthly partitions exist from Soroban activation to now + FUTURE_MONTHS.
pub async fn ensure_time_partitions(
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

/// Ensures the `<table>_default` catch-all partition exists **and is
/// attached**. Required at-least-once on every fresh DB before backfill
/// writes anything, because Postgres routes any `created_at` outside the
/// explicit monthly children to `_default`. Without it, every INSERT on a
/// partitioned parent fails with "no partition of relation found".
///
/// Three explicit branches against the live state, queried up front via
/// `pg_inherits` + `to_regclass`, so the function reaches the right DDL on
/// the first try instead of relying on a SQLSTATE sentinel:
///
/// 1. **Already attached** — no-op.
/// 2. **Exists detached** (rare; only after manual `DETACH PARTITION`) —
///    `ALTER TABLE ... ATTACH PARTITION ... DEFAULT`.
/// 3. **Missing entirely** — `CREATE TABLE ... PARTITION OF ... DEFAULT`.
///
/// `CREATE TABLE IF NOT EXISTS` was tried originally and rejected — it
/// silently swallows the "exists detached" case (no error code raised) and
/// would leave a detached `_default` permanently invisible to inserts.
pub async fn ensure_default_partition(pool: &PgPool, table: &str) -> Result<(), Error> {
    let name = format!("{table}_default");

    let attached: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM pg_inherits
              WHERE inhrelid  = to_regclass($1)
                AND inhparent = to_regclass($2)
         )",
    )
    .bind(&name)
    .bind(table)
    .fetch_one(pool)
    .await?;

    if attached {
        return Ok(());
    }

    let exists_anywhere: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(&name)
        .fetch_one(pool)
        .await?;

    let ddl = if exists_anywhere {
        format!("ALTER TABLE {table} ATTACH PARTITION {name} DEFAULT")
    } else {
        format!("CREATE TABLE {name} PARTITION OF {table} DEFAULT")
    };
    sqlx::query(&ddl).execute(pool).await?;
    Ok(())
}

/// Ensures every partitioned parent in `TIME_PARTITIONED_TABLES` has both
/// its `_default` catch-all and the full monthly children covering
/// `SOROBAN_START → today + FUTURE_MONTHS`. Used by the CLI binary; the
/// Lambda performs the same two steps inline so it can publish per-table
/// CloudWatch dimensions between calls (see `handler` above).
///
/// Returns the total number of monthly children that were either created
/// or reattached across all seven tables — `ensure_time_partitions`
/// increments its counter for both fresh `CREATE` and the recovery
/// `ATTACH PARTITION` branch, so callers should treat the value as "DDL
/// statements run", not "rows newly inserted into pg_class". `_default`
/// is intentionally not in the count: each call resolves to exactly one
/// of three states (already-attached / reattach / create) and the
/// distinction matters only for diagnostics, not the operator-facing
/// total.
pub async fn ensure_all_partitions(pool: &PgPool, today: NaiveDate) -> Result<u32, Error> {
    let mut total_created = 0u32;
    for table in TIME_PARTITIONED_TABLES {
        ensure_default_partition(pool, table).await?;
        let created = ensure_time_partitions(pool, table, today).await?;
        total_created += created;
    }
    Ok(total_created)
}

/// Counts partitions that cover months strictly after today.
pub async fn count_future_partitions(
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
pub async fn get_existing_partitions(
    pool: &PgPool,
    parent_table: &str,
) -> Result<Vec<String>, Error> {
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
pub fn parse_partition_month(name: &str) -> Option<NaiveDate> {
    let y_pos = name.rfind("_y")?;
    let suffix = &name[y_pos + 2..];
    let m_pos = suffix.find('m')?;
    let year: i32 = suffix[..m_pos].parse().ok()?;
    let month: u32 = suffix[m_pos + 1..].parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, 1)
}

// ────────────────────────── Helpers ────────────────────────────

/// Adds N months to a NaiveDate (clamped to 1st of month).
pub fn add_months(date: NaiveDate, months: u32) -> NaiveDate {
    let total_months = date.year() * 12 + date.month() as i32 - 1 + months as i32;
    let year = total_months / 12;
    let month = (total_months % 12) + 1;
    NaiveDate::from_ymd_opt(year, month as u32, 1).unwrap_or(date)
}

// ────────────────────────── Tests ──────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_partition_month_valid() {
        assert_eq!(
            parse_partition_month("soroban_events_appearances_y2026m04"),
            Some(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap())
        );
        assert_eq!(
            parse_partition_month("liquidity_pool_snapshots_y2024m12"),
            Some(NaiveDate::from_ymd_opt(2024, 12, 1).unwrap())
        );
    }

    #[test]
    fn parse_partition_month_invalid() {
        assert_eq!(
            parse_partition_month("soroban_events_appearances_default"),
            None
        );
        assert_eq!(parse_partition_month("random_name"), None);
    }

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

    #[test]
    fn months_to_create_fills_gap() {
        // Existing: only Apr 2026. Today: Apr 2026.
        let existing = vec!["soroban_events_appearances_y2026m04".to_string()];
        let today = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
        let missing = months_to_create("soroban_events_appearances", &existing, today);

        assert!(
            !missing
                .iter()
                .any(|(n, _)| n == "soroban_events_appearances_y2026m04")
        );
        assert!(
            missing
                .iter()
                .any(|(n, _)| n == "soroban_events_appearances_y2024m02")
        );
        assert!(
            missing
                .iter()
                .any(|(n, _)| n == "soroban_events_appearances_y2026m07")
        );
        assert!(
            !missing
                .iter()
                .any(|(n, _)| n == "soroban_events_appearances_y2026m08")
        );
    }

    #[test]
    fn months_to_create_all_exist() {
        let today = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
        let existing: Vec<String> = (2..=6)
            .map(|m| format!("soroban_events_appearances_y2024m{m:02}"))
            .collect();
        let missing = months_to_create("soroban_events_appearances", &existing, today);
        assert!(missing.is_empty());
    }

    #[test]
    fn months_to_create_upper_bound() {
        let today = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
        let existing: Vec<String> = (2..=5)
            .map(|m| format!("soroban_events_appearances_y2024m{m:02}"))
            .collect();
        let missing = months_to_create("soroban_events_appearances", &existing, today);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, "soroban_events_appearances_y2024m06");
    }

    #[test]
    fn post_adr_0027_tables_in_time_partitioned_list() {
        // Regression guard: ADR 0027 moved these three from transaction_id
        // range partitioning to created_at monthly. If any assertion fails,
        // the schema changed again and this module needs updating.
        for table in [
            "transactions",
            "operations_appearances",
            "transaction_participants",
        ] {
            assert!(
                TIME_PARTITIONED_TABLES.contains(&table),
                "missing {table} from TIME_PARTITIONED_TABLES"
            );
        }
    }
}
