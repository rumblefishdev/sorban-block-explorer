//! Typed errors for the backfill runner.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackfillError {
    /// Pool initialization failed — not an indexer error, a config/boot error.
    #[error("database pool init: {0}")]
    Db(#[from] sqlx::Error),

    #[error("indexer: {0}")]
    Indexer(#[from] indexer::handler::HandlerError),

    /// Local filesystem / subprocess I/O failure (create_dir_all, File::create,
    /// read_dir, spawning `aws`, etc.). Constructed via `#[from]`.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// `aws s3 sync` subprocess exited non-zero after all retry attempts were
    /// exhausted. Carries enough context (partition + exit code + stderr tail)
    /// to diagnose without re-running.
    #[error("aws s3 sync failed for partition {partition} (exit {exit_code}): {stderr}")]
    AwsSyncFailed {
        partition: u32,
        exit_code: i32,
        stderr: String,
    },
}
