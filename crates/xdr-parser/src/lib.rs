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

pub mod envelope;
mod xdr_limits;

pub use contract::extract_contract_interfaces;
pub use envelope::InnerTxRef;
pub use error::{ParseError, ParseErrorKind};
pub use event::extract_events;
pub use invocation::{InvocationResult, extract_invocations};
pub use ledger::extract_ledger;
pub use ledger_entry_changes::extract_ledger_entry_changes;
pub use nft::detect_nft_events;
pub use operation::extract_operations;
pub use scval::scval_to_typed_json;
pub use state::{
    detect_nfts, detect_tokens, extract_account_states, extract_contract_deployments,
    extract_liquidity_pools,
};
pub use transaction::extract_transactions;
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
/// Galexie writes files as `{hex}--{start}[-{end}].xdr.zst` where `{hex}` is an
/// 8-character hex prefix and `{start}`/`{end}` are decimal u32 ledger sequences.
/// Single-ledger files omit `-{end}` and return `(start, start)`.
///
/// The suffix **must** be `.xdr.zst` — this must stay in sync with the S3 event
/// filter in `infra/src/lib/stacks/compute-stack.ts` (`{ suffix: '.xdr.zst' }`).
pub fn parse_s3_key(key: &str) -> Result<(u32, u32), ParseError> {
    let filename = key
        .rsplit('/')
        .next()
        .ok_or_else(|| ParseError {
            kind: ParseErrorKind::InvalidS3Key,
            message: format!("no filename in key: {key}"),
            context: None,
        })?
        .strip_suffix(".xdr.zst")
        .ok_or_else(|| ParseError {
            kind: ParseErrorKind::InvalidS3Key,
            message: format!("key does not end with .xdr.zst: {key}"),
            context: None,
        })?;

    // Split on `--` to separate hex prefix from ledger sequence(s).
    let (hex_prefix, ledger_part) = filename.split_once("--").ok_or_else(|| ParseError {
        kind: ParseErrorKind::InvalidS3Key,
        message: format!("filename missing '--' separator: {filename}"),
        context: None,
    })?;

    // Validate hex prefix (Galexie uses 8-char hex, derived from uint32_max - ledger).
    if hex_prefix.len() != 8 || !hex_prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ParseError {
            kind: ParseErrorKind::InvalidS3Key,
            message: format!("invalid hex prefix: {hex_prefix}"),
            context: None,
        });
    }

    // Parse ledger sequence(s): either `{start}` or `{start}-{end}`.
    let (start, end) = if let Some((start_str, end_str)) = ledger_part.split_once('-') {
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
        (start, end)
    } else {
        let seq = ledger_part.parse::<u32>().map_err(|e| ParseError {
            kind: ParseErrorKind::InvalidS3Key,
            message: format!("invalid ledger sequence: {e}"),
            context: None,
        })?;
        (seq, seq)
    };

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

    // Galexie filename format: {hex}--{start}[-{end}].xdr.zst
    // Suffix must match CDK S3 filter in compute-stack.ts: { suffix: '.xdr.zst' }

    #[test]
    fn parse_s3_key_valid() {
        let (start, end) =
            parse_s3_key("FC4DB5FF--62016000-62079999/FC4D8B46--62026937.xdr.zst").unwrap();
        assert_eq!(start, 62026937);
        assert_eq!(end, 62026937);
    }

    #[test]
    fn parse_s3_key_valid_range() {
        let (start, end) =
            parse_s3_key("FC4DB5FF--62016000-62079999/FC4D8B46--62026937-62026938.xdr.zst")
                .unwrap();
        assert_eq!(start, 62026937);
        assert_eq!(end, 62026938);
    }

    #[test]
    fn parse_s3_key_valid_no_path() {
        let (start, end) = parse_s3_key("ABCD1234--100.xdr.zst").unwrap();
        assert_eq!(start, 100);
        assert_eq!(end, 100);
    }

    #[test]
    fn parse_s3_key_invalid_suffix() {
        assert!(parse_s3_key("FC4D8B46--62026937.xdr.zstd").is_err());
    }

    #[test]
    fn parse_s3_key_missing_double_dash() {
        assert!(parse_s3_key("62026937.xdr.zst").is_err());
    }

    #[test]
    fn parse_s3_key_invalid_hex_prefix() {
        assert!(parse_s3_key("GGGGGGGG--62026937.xdr.zst").is_err());
    }

    #[test]
    fn parse_s3_key_short_hex_prefix() {
        assert!(parse_s3_key("ABCD--62026937.xdr.zst").is_err());
    }

    #[test]
    fn parse_s3_key_start_greater_than_end() {
        assert!(parse_s3_key("FC4D8B46--200-100.xdr.zst").is_err());
    }

    #[test]
    fn parse_s3_key_invalid_number() {
        assert!(parse_s3_key("FC4D8B46--abc.xdr.zst").is_err());
    }
}
