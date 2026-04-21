//! backfill-runner — production-grade Stellar pubnet backfill to Postgres.
//!
//! Source: `aws-public-blockchain/v1.1/stellar/ledgers/pubnet/` (unsigned).
//! Sink:   Postgres, ADR 0027 schema, via
//!         `indexer::handler::process::process_ledger` (parse-and-persist).

mod error;
mod ingest;
mod partition;
mod resume;
mod run;
mod source;
mod status;

use clap::{Parser, Subcommand};
use tracing::error;

#[derive(Parser)]
#[command(name = "backfill-runner", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// PostgreSQL connection string.
    #[arg(long, env = "DATABASE_URL", global = true)]
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

        /// Number of ledgers to process in each worker job (reserved).
        #[arg(long, default_value_t = 100)]
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
            chunk_size,
        } => run::execute(&cli.database_url, start, end, chunk_size).await,
        Command::Status { start, end } => status::execute(&cli.database_url, start, end).await,
    };

    if let Err(err) = result {
        error!(error = %err, "backfill-runner exiting with error");
        std::process::exit(1);
    }
}
