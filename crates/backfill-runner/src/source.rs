//! S3 source fetcher for the public Stellar archive.
//!
//! `aws-public-blockchain` is a public bucket — requests are unsigned.
//! We stream `.xdr.zst` objects directly into memory; no local scratch dir.

use aws_config::BehaviorVersion;
use aws_sdk_s3::{Client, config::Region, error::SdkError, operation::get_object::GetObjectError};

use crate::error::BackfillError;
use crate::partition;

/// Build an unsigned S3 client pinned to `us-east-1` (bucket region).
pub async fn build_client() -> Client {
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::from_static("us-east-1"))
        .no_credentials()
        .load()
        .await;
    Client::new(&config)
}

/// Fetch one `.xdr.zst` ledger object into memory. No retries here —
/// the caller layers backoff on top.
pub async fn fetch_ledger(client: &Client, seq: u32) -> Result<Vec<u8>, BackfillError> {
    let p = partition::Partition::from_ledger(seq);
    let key = partition::ledger_key(&p, seq);
    let resp = client
        .get_object()
        .bucket(partition::BUCKET)
        .key(&key)
        .send()
        .await
        .map_err(|err| match &err {
            SdkError::ServiceError(svc) if matches!(svc.err(), GetObjectError::NoSuchKey(_)) => {
                BackfillError::S3NotFound { key: key.clone() }
            }
            _ => BackfillError::S3Get {
                key: key.clone(),
                source: Box::new(err),
            },
        })?;

    resp.body
        .collect()
        .await
        .map(|agg| agg.to_vec())
        .map_err(|e| BackfillError::S3Body {
            key,
            source: Box::new(e),
        })
}
