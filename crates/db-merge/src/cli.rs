//! CLI surface for db-merge. Three subcommands per task 0186:
//! `ingest`, `finalize`, `diff`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "db-merge", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[arg(long, short, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Ingest one snapshot into the merge target. Run once per snapshot,
    /// chronologically oldest-first.
    Ingest {
        /// Path to a `pg_dump --format=custom` snapshot file.
        snapshot: PathBuf,

        /// Merge target Postgres URL (the DB receiving merged data).
        #[arg(long, env = "DB_MERGE_TARGET_URL")]
        target_url: String,

        /// Ephemeral snapshot-source Postgres URL — the container that
        /// receives `pg_restore` and is exposed via postgres_fdw to the
        /// target. Reset on every invocation.
        #[arg(long, env = "DB_MERGE_SNAPSHOT_SOURCE_URL")]
        snapshot_source_url: String,

        /// Bypass the chronological-only ledger-range check. Required for
        /// T4 idempotency runs (replaying a snapshot already merged).
        /// Without this flag, `merge ingest` aborts when source ledger
        /// range precedes-or-overlaps the target's existing range.
        #[arg(long)]
        allow_overlap: bool,
    },

    /// Run post-merge finalization (Step 13 + Step 14 from task 0186):
    /// rebuild `nfts.current_owner_*` and `setval` all 7 sequences.
    /// Idempotent. Run once after the last `ingest`.
    Finalize {
        #[arg(long, env = "DB_MERGE_TARGET_URL")]
        target_url: String,
    },

    /// Per-table normalized-hash diff between two databases. Produces a
    /// 25-row table with row counts + md5 per table on natural-key
    /// projections (surrogate ids and `search_vector` excluded).
    Diff {
        #[arg(long)]
        left: String,

        #[arg(long)]
        right: String,
    },
}
