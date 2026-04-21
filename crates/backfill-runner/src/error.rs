//! Typed errors for the backfill runner.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackfillError {
    /// Pool initialization failed — not an indexer error, a config/boot error.
    #[error("database pool init: {0}")]
    Db(#[from] sqlx::Error),

    #[error("indexer: {0}")]
    Indexer(#[from] indexer::handler::HandlerError),

    /// S3 `GetObject` failed (network, auth, throttling, etc.).
    #[error("s3 get_object {key}: {source}")]
    S3Get {
        key: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// S3 returned 404 for a ledger key.
    #[error("s3 ledger not found: {key}")]
    S3NotFound { key: String },

    /// Reading the body stream failed after `GetObject` succeeded.
    #[error("s3 body read {key}: {source}")]
    S3Body {
        key: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Run finished but some sequences were absent from the public archive.
    /// Not fatal per-ledger — aggregated at the end so the runner exits non-zero.
    #[error("{} sequences missing from archive", .0.len())]
    MissingLedgers(Vec<u32>),
}
