//! XDR parser for the Soroban block explorer indexer.
//!
//! Deserializes `LedgerCloseMeta` payloads from Galexie S3 exports and extracts
//! structured ledger and transaction data for PostgreSQL persistence.
//!
//! This is the sole XDR parsing path (ADR-0004: Rust-only, ingestion-time).

pub mod contract;
pub mod error;
pub mod event;
pub mod invocation;
pub mod ledger;
pub mod ledger_entry_changes;
pub mod memo;
pub mod nft;
pub mod operation;
pub mod scval;
pub mod state;
pub mod transaction;
pub mod types;

pub(crate) mod envelope;
mod xdr_limits;

pub use contract::extract_contract_interfaces;
pub use envelope::InnerTxRef;
pub use error::{ParseError, ParseErrorKind};
pub use event::extract_events;
pub use invocation::{extract_invocations, InvocationResult};
pub use ledger::extract_ledger;
pub use ledger_entry_changes::extract_ledger_entry_changes;
pub use nft::detect_nft_events;
pub use state::{
    detect_nfts, detect_tokens, extract_account_states, extract_contract_deployments,
    extract_liquidity_pools,
};
pub use operation::extract_operations;
pub use transaction::extract_transactions;
pub use scval::scval_to_typed_json;
pub use types::{
    ContractFunction, ExtractedAccountState, ExtractedContractDeployment,
    ExtractedContractInterface, ExtractedEvent, ExtractedInvocation, ExtractedLedger,
    ExtractedLedgerEntryChange, ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot,
    ExtractedNft, ExtractedOperation, ExtractedToken, ExtractedTransaction, NftEvent,
};

use stellar_xdr::curr::{LedgerCloseMetaBatch, ReadXdr};

/// Maximum decompressed size (64 MiB). Galexie batches are typically 2-5 MiB.
const MAX_DECOMPRESSED_SIZE: usize = 64 * 1024 * 1024;

/// Decompress a zstd-compressed XDR payload with size limit.
pub fn decompress_zstd(compressed: &[u8]) -> Result<Vec<u8>, ParseError> {
    let decompressed = zstd::decode_all(compressed).map_err(|e| ParseError {
        kind: ParseErrorKind::DecompressionFailed,
        message: e.to_string(),
        context: None,
    })?;
    if decompressed.len() > MAX_DECOMPRESSED_SIZE {
        return Err(ParseError {
            kind: ParseErrorKind::DecompressionFailed,
            message: format!(
                "decompressed size {} exceeds limit {}",
                decompressed.len(),
                MAX_DECOMPRESSED_SIZE
            ),
            context: None,
        });
    }
    Ok(decompressed)
}

/// Deserialize a `LedgerCloseMetaBatch` from raw XDR bytes.
pub fn deserialize_batch(xdr_bytes: &[u8]) -> Result<LedgerCloseMetaBatch, ParseError> {
    let limits = xdr_limits::deserialization_limits(xdr_bytes.len());
    LedgerCloseMetaBatch::from_xdr(xdr_bytes, limits).map_err(|e| ParseError {
        kind: ParseErrorKind::XdrDeserializationFailed,
        message: e.to_string(),
        context: None,
    })
}

/// Parse the filename portion of an S3 object key to extract the ledger sequence range.
///
/// Validates that the filename ends with `.xdr.zstd` and contains `{start}-{end}`.
/// Does not enforce a specific path prefix.
pub fn parse_s3_key(key: &str) -> Result<(u32, u32), ParseError> {
    let filename = key
        .rsplit('/')
        .next()
        .ok_or_else(|| ParseError {
            kind: ParseErrorKind::InvalidS3Key,
            message: format!("no filename in key: {key}"),
            context: None,
        })?
        .strip_suffix(".xdr.zstd")
        .ok_or_else(|| ParseError {
            kind: ParseErrorKind::InvalidS3Key,
            message: format!("key does not end with .xdr.zstd: {key}"),
            context: None,
        })?;

    let (start_str, end_str) = filename.split_once('-').ok_or_else(|| ParseError {
        kind: ParseErrorKind::InvalidS3Key,
        message: format!("filename missing dash separator: {filename}"),
        context: None,
    })?;

    let start = start_str.parse::<u32>().map_err(|e| ParseError {
        kind: ParseErrorKind::InvalidS3Key,
        message: format!("invalid start sequence: {e}"),
        context: None,
    })?;
    let end = end_str.parse::<u32>().map_err(|e| ParseError {
        kind: ParseErrorKind::InvalidS3Key,
        message: format!("invalid end sequence: {e}"),
        context: None,
    })?;

    if start > end {
        return Err(ParseError {
            kind: ParseErrorKind::InvalidS3Key,
            message: format!("start sequence {start} > end sequence {end}"),
            context: None,
        });
    }

    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_s3_key_valid() {
        let (start, end) =
            parse_s3_key("stellar-ledger-data/ledgers/61827430-61827440.xdr.zstd").unwrap();
        assert_eq!(start, 61827430);
        assert_eq!(end, 61827440);
    }

    #[test]
    fn parse_s3_key_invalid_suffix() {
        assert!(parse_s3_key("ledgers/100-200.xdr").is_err());
    }

    #[test]
    fn parse_s3_key_missing_dash() {
        assert!(parse_s3_key("ledgers/100200.xdr.zstd").is_err());
    }

    #[test]
    fn parse_s3_key_start_greater_than_end() {
        assert!(parse_s3_key("ledgers/200-100.xdr.zstd").is_err());
    }
}
