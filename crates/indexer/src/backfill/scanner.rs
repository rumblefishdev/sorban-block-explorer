use aws_sdk_s3::Client as S3Client;
use tracing::{debug, info, warn};

use crate::range::LedgerRange;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("S3 ListObjectsV2 failed: {0}")]
    S3List(String),
}

/// Scan S3 for existing XDR files and return the ledger ranges they cover.
///
/// Lists all objects under `prefix` in the bucket, parses each key with
/// `xdr_parser::parse_s3_key`, and returns sorted ranges.
pub async fn scan_existing_ranges(
    client: &S3Client,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<LedgerRange>, ScanError> {
    let mut ranges = Vec::new();
    let mut continuation_token: Option<String> = None;
    let mut page = 0u32;

    loop {
        let mut request = client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix)
            .max_keys(1000);

        if let Some(token) = continuation_token.take() {
            request = request.continuation_token(token);
        }

        let response = request
            .send()
            .await
            .map_err(|e| ScanError::S3List(e.to_string()))?;

        let contents = response.contents();
        for obj in contents {
            if let Some(key) = obj.key() {
                match xdr_parser::parse_s3_key(key) {
                    Ok((start, end)) => {
                        ranges.push(LedgerRange::new(start, end));
                    }
                    Err(e) => {
                        debug!(key, error = %e, "skipping non-ledger S3 key");
                    }
                }
            }
        }

        page += 1;
        if page.is_multiple_of(10) {
            info!(
                pages = page,
                keys_found = ranges.len(),
                "S3 scan in progress"
            );
        }

        match response.next_continuation_token() {
            Some(token) => continuation_token = Some(token.to_string()),
            None => break,
        }
    }

    if ranges.is_empty() {
        warn!(bucket, prefix, "no existing ledger files found in S3");
    } else {
        info!(files = ranges.len(), "S3 scan complete");
    }

    ranges.sort_by_key(|r| r.start);
    Ok(ranges)
}
