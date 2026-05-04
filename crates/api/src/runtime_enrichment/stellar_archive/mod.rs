//! Read-time fetch of raw `.xdr.zst` ledger files from the public Stellar archive.
//!
//! Implements the read-path component of ADR 0029: heavy fields (memo, signatures,
//! full event topics/data, XDR blobs) not persisted in the DB are pulled on-demand
//! from `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/` at request time.
//!
//! Callers pass a slice of ledger sequences and receive
//! `Vec<Result<LedgerCloseMeta, FetchError>>` in input order, so each requested
//! ledger may succeed or fail independently. Downloads run concurrently up to
//! `MAX_CONCURRENT_FETCHES`. No caching — follow-up task if needed.

pub mod dto;
pub mod extractors;
pub mod key;
pub mod merge;

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use futures::stream::{self, StreamExt};
use std::time::Duration;
use stellar_xdr::curr::LedgerCloseMeta;
use thiserror::Error;
use tracing::instrument;

use self::key::build_s3_key;

/// Public Stellar data archive bucket. No credentials required.
pub const PUBLIC_ARCHIVE_BUCKET: &str = "aws-public-blockchain";

/// S3 key prefix inside the bucket for pubnet ledgers.
pub const PUBLIC_ARCHIVE_PREFIX: &str = "v1.1/stellar/ledgers/pubnet";

/// Default per-request budget for public-archive S3 GETs. Chosen so that an
/// end-to-end E3/E14 request completes well under API Gateway's 29s limit
/// even with a retry or fallback upstream.
pub const DEFAULT_S3_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of concurrent in-flight GETs issued by `fetch_ledgers`.
/// Caps CPU/connection spikes when a caller passes a large slice (e.g.,
/// a full E14 page whose events reference many distinct ledgers).
pub const MAX_CONCURRENT_FETCHES: usize = 16;

/// Build a timeout config applied to every public-archive S3 request.
pub fn default_timeout_config() -> TimeoutConfig {
    TimeoutConfig::builder()
        .operation_timeout(DEFAULT_S3_OPERATION_TIMEOUT)
        .operation_attempt_timeout(DEFAULT_S3_OPERATION_TIMEOUT)
        .build()
}

/// Errors returned by the fetcher.
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("ledger {seq} not found in public archive")]
    NotFound { seq: u32 },

    #[error("S3 error fetching ledger {seq}: {source}")]
    S3 {
        seq: u32,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("decompression failed for ledger {seq}: {source}")]
    Decompress {
        seq: u32,
        #[source]
        source: xdr_parser::ParseError,
    },

    #[error("XDR deserialization failed for ledger {seq}: {source}")]
    Deserialize {
        seq: u32,
        #[source]
        source: xdr_parser::ParseError,
    },

    #[error("empty batch returned for ledger {seq}")]
    EmptyBatch { seq: u32 },
}

/// Fetches raw `.xdr.zst` ledger files from the public Stellar archive.
///
/// Stateless and cheap to clone (`S3Client` is `Arc`-backed internally).
/// Create once per Lambda via shared state; reuse across requests.
#[derive(Clone)]
pub struct StellarArchiveFetcher {
    client: S3Client,
}

impl StellarArchiveFetcher {
    /// Construct a fetcher from a pre-configured unsigned S3 client.
    pub fn new(client: S3Client) -> Self {
        Self { client }
    }

    /// Fetch, decompress, and deserialize a single ledger.
    ///
    /// Returns the first `LedgerCloseMeta` in the batch — the public archive
    /// writes one ledger per file, so there is never more than one.
    #[instrument(skip(self), fields(ledger_seq = seq))]
    pub async fn fetch_ledger(&self, seq: u32) -> Result<LedgerCloseMeta, FetchError> {
        let key = format!("{PUBLIC_ARCHIVE_PREFIX}/{}", build_s3_key(seq));

        let compressed = self.download(seq, &key).await?;
        let xdr_bytes = xdr_parser::decompress_zstd(compressed.as_ref())
            .map_err(|source| FetchError::Decompress { seq, source })?;
        let batch = xdr_parser::deserialize_batch(&xdr_bytes)
            .map_err(|source| FetchError::Deserialize { seq, source })?;

        let metas: Vec<LedgerCloseMeta> = batch.ledger_close_metas.into();
        metas
            .into_iter()
            .next()
            .ok_or(FetchError::EmptyBatch { seq })
    }

    /// Fetch multiple ledgers concurrently. Results are returned in input order.
    ///
    /// Concurrency is capped at `MAX_CONCURRENT_FETCHES` to prevent connection
    /// and CPU spikes when callers pass a large slice. On any per-ledger
    /// failure the corresponding slot contains `Err` — callers decide how to
    /// handle partial failure (e.g. fail fast, degrade gracefully).
    #[instrument(skip(self, seqs), fields(count = seqs.len()))]
    pub async fn fetch_ledgers(&self, seqs: &[u32]) -> Vec<Result<LedgerCloseMeta, FetchError>> {
        stream::iter(seqs.iter().copied())
            .map(|seq| self.fetch_ledger(seq))
            .buffered(MAX_CONCURRENT_FETCHES)
            .collect()
            .await
    }

    async fn download(&self, seq: u32, key: &str) -> Result<bytes::Bytes, FetchError> {
        let resp = self
            .client
            .get_object()
            .bucket(PUBLIC_ARCHIVE_BUCKET)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                // Only promote to `NotFound` for a genuine service-level 404.
                // Other failure modes (timeouts, dispatch, credential
                // resolution) keep the original SDK error as the source so
                // callers/log consumers retain context.
                if matches!(e.as_service_error(), Some(svc) if svc.is_no_such_key()) {
                    FetchError::NotFound { seq }
                } else {
                    FetchError::S3 {
                        seq,
                        source: Box::new(e),
                    }
                }
            })?;

        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| FetchError::S3 {
                seq,
                source: Box::new(e),
            })?
            .into_bytes();

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_config::BehaviorVersion;

    /// Build an unsigned S3 client for anonymous access to the public archive.
    async fn unsigned_client() -> S3Client {
        let config = aws_config::defaults(BehaviorVersion::latest())
            .no_credentials()
            .region("us-east-2") // aws-public-blockchain is in us-east-2
            .timeout_config(default_timeout_config())
            .load()
            .await;
        S3Client::new(&config)
    }

    /// End-to-end fetch of a known Soroban-era ledger from the public archive.
    ///
    /// Ignored by default — requires network access + writes no state. Run with:
    ///   `cargo test --package api -- --ignored runtime_enrichment::stellar_archive::tests::fetch_single_ledger_from_archive`
    #[tokio::test]
    #[ignore = "requires network access to aws-public-blockchain"]
    async fn fetch_single_ledger_from_archive() {
        use stellar_xdr::curr::LedgerCloseMeta;

        let fetcher = StellarArchiveFetcher::new(unsigned_client().await);
        // First Soroban-era ledger per backfill-bench how-to-run.
        let seq = 50_457_424_u32;

        let meta = fetcher.fetch_ledger(seq).await.expect("fetch failed");

        let got_seq = match &meta {
            LedgerCloseMeta::V0(v) => v.ledger_header.header.ledger_seq,
            LedgerCloseMeta::V1(v) => v.ledger_header.header.ledger_seq,
            LedgerCloseMeta::V2(v) => v.ledger_header.header.ledger_seq,
        };
        assert_eq!(got_seq, seq, "ledger sequence mismatch");
    }

    /// Batch fetch — verifies concurrent fetch path.
    #[tokio::test]
    #[ignore = "requires network access to aws-public-blockchain"]
    async fn fetch_multiple_ledgers_concurrently_from_stellar_archive() {
        let fetcher = StellarArchiveFetcher::new(unsigned_client().await);
        let seqs = [50_457_424_u32, 50_457_425, 50_457_426];

        let results = fetcher.fetch_ledgers(&seqs).await;

        assert_eq!(results.len(), seqs.len());
        for (i, result) in results.iter().enumerate() {
            assert!(result.is_ok(), "ledger {} failed: {:?}", seqs[i], result);
        }
    }

    /// Scan a sequence of ledgers for one containing at least one transaction
    /// and return (ledger_meta, first_tx_hash). Gives the E3/E14 end-to-end
    /// tests a non-empty ledger without hard-coding a tx hash that may
    /// disappear if the public archive ever changes format.
    async fn find_ledger_with_tx(
        fetcher: &StellarArchiveFetcher,
        start: u32,
        window: u32,
    ) -> (LedgerCloseMeta, String) {
        for seq in start..start + window {
            let Ok(meta) = fetcher.fetch_ledger(seq).await else {
                continue;
            };
            let ledger = xdr_parser::extract_ledger(&meta);
            let net_id = xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE);
            let txs =
                xdr_parser::extract_transactions(&meta, ledger.sequence, ledger.closed_at, &net_id);
            if let Some(first_tx) = txs.iter().find(|t| !t.parse_error) {
                return (meta, first_tx.hash.clone());
            }
        }
        panic!(
            "no ledger with a parseable tx in range [{start}, {}]",
            start + window
        );
    }

    /// End-to-end E3 pipeline test:
    /// fetch real ledger → extract heavy fields for a real transaction hash
    /// → verify the heavy-field shape looks sensible.
    #[tokio::test]
    #[ignore = "requires network access to aws-public-blockchain"]
    async fn extract_e3_heavy_fields_from_real_stellar_tx() {
        use super::extractors::extract_e3_heavy;

        let fetcher = StellarArchiveFetcher::new(unsigned_client().await);
        // Start at first Soroban ledger; scan up to 20 ledgers for one with txs.
        let (meta, tx_hash) = find_ledger_with_tx(&fetcher, 50_457_424, 20).await;

        let net_id = xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE);
        let heavy = extract_e3_heavy(&meta, &tx_hash, &net_id).expect("tx hash found in ledger");

        // XDR blobs must be populated for a non-parse-error tx.
        assert!(
            heavy.envelope_xdr.is_some(),
            "envelope_xdr missing: {tx_hash}"
        );
        assert!(heavy.result_xdr.is_some(), "result_xdr missing: {tx_hash}");
        // Signatures: at least one on a genuine Stellar tx.
        assert!(
            !heavy.signatures.is_empty(),
            "expected ≥1 signature on tx {tx_hash}"
        );
        // Every signature should be non-empty hex.
        for sig in &heavy.signatures {
            assert_eq!(sig.hint.len(), 8, "hint must be 4 bytes hex");
            assert!(!sig.signature.is_empty(), "empty signature bytes");
        }
    }

    /// E3 heavy extraction should return None for an unknown tx hash.
    #[tokio::test]
    #[ignore = "requires network access to aws-public-blockchain"]
    async fn e3_extractor_return_none_for_unknown_tx_hash() {
        use super::extractors::extract_e3_heavy;

        let fetcher = StellarArchiveFetcher::new(unsigned_client().await);
        let meta = fetcher.fetch_ledger(50_457_424).await.unwrap();

        // Not a real tx hash in this ledger.
        let fake_hash = "deadbeef".repeat(8);
        let net_id = xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE);
        assert!(extract_e3_heavy(&meta, &fake_hash, &net_id).is_none());
    }

    /// End-to-end E14 pipeline test:
    /// fetch real ledger → scan it for a Soroban contract → extract heavy
    /// fields (full topics + data) for that contract → verify the shape.
    ///
    /// Scans a small window since not every ledger has contract events.
    #[tokio::test]
    #[ignore = "requires network access to aws-public-blockchain"]
    async fn extract_e14_heavy_fields_from_real_stellar_contract() {
        use super::extractors::extract_e14_heavy;

        let fetcher = StellarArchiveFetcher::new(unsigned_client().await);

        // Scan post-Soroban-launch window until we find a ledger with a
        // contract event; Soroban adoption ramped up over many ledgers.
        for seq in 55_000_000_u32..55_000_200 {
            let Ok(meta) = fetcher.fetch_ledger(seq).await else {
                continue;
            };
            let ledger = xdr_parser::extract_ledger(&meta);
            let net_id = xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE);
            let txs =
                xdr_parser::extract_transactions(&meta, ledger.sequence, ledger.closed_at, &net_id);
            let tx_metas = super::extractors::collect_tx_metas(&meta);

            // Find any contract emitting events in this ledger.
            let mut contract_id: Option<String> = None;
            for (i, tx) in txs.iter().enumerate() {
                if tx.parse_error {
                    continue;
                }
                let Some(tm) = tx_metas.get(i).copied() else {
                    continue;
                };
                let events =
                    xdr_parser::extract_events(tm, &tx.hash, ledger.sequence, ledger.closed_at);
                if let Some(cid) = events.into_iter().find_map(|e| e.contract_id) {
                    contract_id = Some(cid);
                    break;
                }
            }

            let Some(cid) = contract_id else { continue };

            let heavy = extract_e14_heavy(&meta, &cid, &net_id);
            assert!(
                !heavy.is_empty(),
                "E14 heavy extraction returned no events for contract {cid} in ledger {seq}"
            );
            for event in &heavy {
                assert!(
                    !event.topics.is_empty(),
                    "event {} has empty topics",
                    event.event_index
                );
                assert!(
                    !event.transaction_hash.is_empty(),
                    "event {} missing transaction_hash",
                    event.event_index
                );
            }
            return;
        }
        panic!("no contract events found in scan window");
    }
}
