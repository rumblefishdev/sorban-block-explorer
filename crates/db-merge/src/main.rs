//! db-merge — multi-laptop backfill snapshot merge tool.
//!
//! Implements the playbook from ADR 0040 and the implementation plan in
//! task 0186. Run `db-merge ingest` once per snapshot
//! (chronologically oldest-first), then `db-merge finalize` once.

mod backup;
mod batcher;
mod cli;
mod diff;
mod error;
mod fdw;
mod finalize;
mod ingest;
mod preflight;
mod snapshot_source;
mod steps;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let filter = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    match cli.command {
        Command::Ingest {
            snapshot,
            target_url,
            snapshot_source_url,
            allow_overlap,
        } => {
            ingest::execute(&snapshot, &target_url, &snapshot_source_url, allow_overlap)
                .await
                .expect("ingest failed");
        }
        Command::Finalize { target_url } => {
            finalize::execute(&target_url)
                .await
                .expect("finalize failed");
        }
        Command::Diff { left, right } => {
            diff::execute(&left, &right).await.expect("diff failed");
        }
    }
}
