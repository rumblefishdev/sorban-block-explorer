//! Lambda handler for the Ledger Processor.
//!
//! Receives S3 PutObject events, downloads and decompresses XDR files,
//! orchestrates the four parsing stages, and wraps all writes for a single
//! ledger in one atomic database transaction.

mod convert;
mod persist;
pub mod process;

use aws_sdk_s3::Client as S3Client;
use lambda_runtime::{Error, LambdaEvent};
use serde::Deserialize;
use sqlx::PgPool;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// S3 event types (subset of the full Lambda S3 event schema)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct S3Event {
    #[serde(rename = "Records")]
    pub records: Vec<S3EventRecord>,
}

#[derive(Debug, Deserialize)]
pub struct S3EventRecord {
    pub s3: S3Entity,
}

#[derive(Debug, Deserialize)]
pub struct S3Entity {
    pub bucket: S3Bucket,
    pub object: S3Object,
}

#[derive(Debug, Deserialize)]
pub struct S3Bucket {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct S3Object {
    pub key: String,
}

// ---------------------------------------------------------------------------
// Handler error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    #[error("S3 download failed: {0}")]
    S3Download(String),
    #[error("parse error: {0}")]
    Parse(#[from] xdr_parser::ParseError),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ---------------------------------------------------------------------------
// Shared state passed to every invocation
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct HandlerState {
    pub s3_client: S3Client,
    pub db_pool: PgPool,
}

// ---------------------------------------------------------------------------
// Lambda entry point
// ---------------------------------------------------------------------------

/// Top-level Lambda handler. Processes each S3 record independently.
pub async fn handler(event: LambdaEvent<S3Event>, state: &HandlerState) -> Result<(), Error> {
    let (payload, _ctx) = event.into_parts();

    let total = payload.records.len();
    let mut skipped = 0usize;

    for record in &payload.records {
        let bucket = &record.s3.bucket.name;
        // S3 event keys are URL-encoded (e.g. slashes as %2F, spaces as +).
        let key = percent_encoding::percent_decode_str(&record.s3.object.key)
            .decode_utf8_lossy()
            .into_owned();

        info!(bucket, key = key.as_str(), "processing S3 record");

        // Validate S3 key pattern (must match Galexie filename format).
        let ledger_range = match xdr_parser::parse_s3_key(&key) {
            Ok(range) => range,
            Err(e) => {
                warn!(bucket, key = key.as_str(), error = %e, "skipping non-matching S3 key");
                skipped += 1;
                continue;
            }
        };

        match process_s3_object(state, bucket, &key, ledger_range).await {
            Ok(()) => {
                info!(
                    bucket,
                    key = key.as_str(),
                    start = ledger_range.0,
                    end = ledger_range.1,
                    "S3 record processed",
                );
            }
            Err(e) => {
                error!(bucket, key = key.as_str(), error = %e, "failed to process S3 record");
                return Err(e.into());
            }
        }
    }

    if skipped > 0 && skipped == total {
        error!(
            total,
            skipped, "all S3 records skipped by parse_s3_key — no data persisted"
        );
    }

    Ok(())
}

/// Download, decompress, parse, and persist one S3 object containing a batch of ledgers.
async fn process_s3_object(
    state: &HandlerState,
    bucket: &str,
    key: &str,
    _ledger_range: (u32, u32),
) -> Result<(), HandlerError> {
    // Step 1: Download from S3
    let compressed = download_s3_object(&state.s3_client, bucket, key).await?;

    // Step 2: Decompress
    let xdr_bytes = xdr_parser::decompress_zstd(&compressed)?;

    // Step 3: Deserialize batch
    let batch = xdr_parser::deserialize_batch(&xdr_bytes)?;

    // Step 4: Process each ledger in the batch
    for ledger_meta in batch.ledger_close_metas.iter() {
        if let Err(e) = process::process_ledger(ledger_meta, &state.db_pool).await {
            error!(
                key,
                error = %e,
                "ledger processing failed — returning error for Lambda retry"
            );
            return Err(e);
        }
    }

    Ok(())
}

/// Download an object from S3.
async fn download_s3_object(
    client: &S3Client,
    bucket: &str,
    key: &str,
) -> Result<Vec<u8>, HandlerError> {
    let resp = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| HandlerError::S3Download(e.to_string()))?;

    let bytes = resp
        .body
        .collect()
        .await
        .map_err(|e| HandlerError::S3Download(e.to_string()))?
        .into_bytes()
        .to_vec();

    Ok(bytes)
}
