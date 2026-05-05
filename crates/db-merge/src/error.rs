//! Typed errors for db-merge.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MergeError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("subprocess `{cmd}` failed (exit {exit:?}):\n{stderr}")]
    Subprocess {
        cmd: String,
        exit: Option<i32>,
        stderr: String,
    },

    #[error("preflight checks failed:\n{0}")]
    Preflight(String),

    #[error(
        "another db-merge ingest is in progress on this target (advisory lock held). \
         Wait for it to finish, or release the lock manually if it's stale."
    )]
    ConcurrentInvocation,
}
