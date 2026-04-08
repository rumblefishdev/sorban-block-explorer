//! Backfill orchestrator for the Soroban block explorer.
//!
//! Launches Galexie ECS Fargate tasks across non-overlapping ledger ranges to
//! backfill historical data from Soroban mainnet activation to a target ledger.

mod config;
mod orchestrator;
mod range;
mod runner;
mod scanner;

use clap::Parser;
use tracing::{error, info};

use config::BackfillConfig;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let config = BackfillConfig::parse();

    if let Err(e) = config.validate() {
        error!(error = %e, "invalid configuration");
        std::process::exit(1);
    }

    info!(
        start = config.start,
        end = config.end,
        batch_size = config.batch_size,
        concurrency = config.concurrency,
        env_name = config.env_name,
        cluster = config.cluster_name(),
        task_def = config.task_def_family(),
        bucket = config.bucket_name(),
        dry_run = config.dry_run,
        "backfill orchestrator starting"
    );

    match orchestrator::run(&config).await {
        Ok(summary) if summary.has_failures() => {
            error!(
                succeeded = summary.succeeded,
                failed = summary.failed,
                "backfill completed with failures"
            );
            std::process::exit(1);
        }
        Ok(summary) => {
            info!(
                succeeded = summary.succeeded,
                skipped = summary.skipped,
                "backfill completed successfully"
            );
        }
        Err(e) => {
            error!(error = %e, "backfill orchestration failed");
            std::process::exit(1);
        }
    }
}
