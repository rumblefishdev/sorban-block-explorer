//! Error type for the per-kind enrichment functions.
//!
//! Two-variant split mirrors the worker's retry semantics: every error
//! the worker handler returns becomes an SQS retry, every `Ok(())`
//! acks the message. So the boundary is "is it worth retrying":
//!
//! - [`EnrichError::Database`] — DB / RDS Proxy / connection issue. SQS
//!   retries until the DLQ threshold; if the cluster is genuinely down
//!   the messages eventually reach the DLQ and the alarm fires.
//! - [`EnrichError::Transient`] — network-layer or 5xx fetch failure;
//!   the issuer's host might come back. SQS retries.
//!
//! Permanent fetch failures (4xx, malformed TOML, no matching
//! `CURRENCIES[]` row, missing `home_domain`) do **not** surface as
//! `EnrichError`. The enrich function writes the empty-string sentinel
//! `''` to the target column itself and returns `Ok(())` so the
//! message is acked and never retried.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnrichError {
    /// Database error (write, read, connection). Worth retrying — SQS
    /// will redeliver per `redrivePolicy.maxReceiveCount` and the DLQ
    /// alarm catches sustained outages.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Transient fetch failure — network-layer (no HTTP status), TCP /
    /// TLS / DNS errors, or a 5xx response from the issuer. The
    /// issuer's host may recover; SQS retries.
    #[error("transient SEP-1 fetch error: {0}")]
    Transient(String),
}
