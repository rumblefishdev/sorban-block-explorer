//! Local / staging CLI for partition management.
//!
//! Same `ensure_all_partitions` code path as the Lambda — no duplication.
//! Used to bootstrap monthly children on docker DBs (where no Lambda runs)
//! and as the one-shot first invocation on a fresh staging RDS before the
//! EventBridge cron takes over.
//!
//! Usage:
//!   DATABASE_URL=postgres://... cargo run -p db-partition-mgmt --bin cli

use chrono::{NaiveDate, Utc};
use clap::Parser;
use sqlx::postgres::PgPoolOptions;

use db_partition_mgmt::{TIME_PARTITIONED_TABLES, ensure_all_partitions};

#[derive(Parser)]
#[command(name = "db-partition-mgmt-cli", about)]
struct Cli {
    /// PostgreSQL connection string.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Override "today" — useful for dry-runs against a specific reference
    /// date. Format: YYYY-MM-DD. Defaults to UTC today.
    #[arg(long)]
    today: Option<NaiveDate>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let today = cli.today.unwrap_or_else(|| Utc::now().naive_utc().date());

    // `max_connections=1` matches the Lambda — partition DDL is serialized
    // anyway, so a single connection is enough and keeps the local docker
    // pool unstressed.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&cli.database_url)
        .await?;

    let total_created = match ensure_all_partitions(&pool, today).await {
        Ok(n) => n,
        Err(err) => {
            // Postgres 23514 = `check_violation`. The partition-management
            // subsystem returns it when the `_default` catch-all already
            // contains rows whose `created_at` falls into a month we're
            // about to add — adding the child would force a rewrite that
            // PG aborts to preserve invariants. The error text from PG is
            // descriptive but doesn't tell operators what to do; surface
            // a one-line remediation hint and re-raise so the exit code
            // is still non-zero.
            if err.to_string().contains("23514") || err.to_string().contains("would be violated") {
                eprintln!(
                    "\nhint: a `_default` partition contains rows in a month we tried to \
                     add. On a smoke/scratch DB the simplest fix is `TRUNCATE TABLE \
                     <parent> CASCADE`; on staging/prod, detach & migrate the rows out \
                     of `_default` first. See lore/3-wiki/backfill-execution-plan.md."
                );
            }
            pool.close().await;
            return Err(err);
        }
    };

    println!(
        "ensured {} partitioned tables; created {} new monthly children for {}",
        TIME_PARTITIONED_TABLES.len(),
        total_created,
        today
    );

    pool.close().await;
    Ok(())
}
