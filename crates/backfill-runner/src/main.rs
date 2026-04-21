//! backfill-runner — production-grade Stellar pubnet backfill to Postgres.
//!
//! Source: `aws-public-blockchain/v1.1/stellar/ledgers/pubnet/` (unsigned).
//! Sink:   Postgres, ADR 0027 schema, via
//!         `indexer::handler::process::process_ledger` (parse-and-persist).

mod error;
mod ingest;
mod partition;
mod run;
mod source;

use clap::{Parser, Subcommand};
use tracing::{error, info};

/// Default worker count. TODO: tune after measurement run (see README).
const DEFAULT_WORKERS: usize = 4;

/// Default chunk size (ledgers per worker job).
const DEFAULT_CHUNK_SIZE: u32 = 100;

#[derive(Parser)]
#[command(name = "backfill-runner", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// PostgreSQL connection string.
    #[arg(
        long,
        env = "DATABASE_URL",
        // default_value = "postgres://postgres:postgres@127.0.0.1:5432/soroban_block_explorer",
        global = true
    )]
    database_url: String,
}

#[derive(Subcommand)]
enum Command {
    /// Run the backfill for a sequence range.
    Run {
        /// First ledger sequence (inclusive).
        #[arg(long)]
        start: u32,

        /// Last ledger sequence (inclusive).
        #[arg(long)]
        end: u32,

        /// Number of concurrent worker tasks.
        #[arg(long, default_value_t = DEFAULT_WORKERS)]
        workers: usize,

        /// Ledgers per worker job.
        #[arg(long, default_value_t = DEFAULT_CHUNK_SIZE)]
        chunk_size: u32,
    },

    /// Report ingested / missing ledgers for a range.
    Status {
        /// First ledger sequence (inclusive).
        #[arg(long)]
        start: u32,

        /// Last ledger sequence (inclusive).
        #[arg(long)]
        end: u32,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let cli = Cli::parse();

    let result = match cli.command {
        Command::Run {
            start,
            end,
            workers,
            chunk_size,
        } => run::execute(&cli.database_url, start, end, workers, chunk_size).await,
        Command::Status { start, end } => {
            info!(start, end, "status subcommand — Phase D stub");
            todo!("Phase D: status subcommand")
        }
    };

    if let Err(err) = result {
        error!(error = %err, "backfill-runner exiting with error");
        std::process::exit(1);
    }
}
