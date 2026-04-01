//! Ledger header extraction from LedgerCloseMeta.

use sha2::{Digest, Sha256};
use stellar_xdr::curr::*;

use crate::error::{ErrorContext, ParseError, ParseErrorKind};
use crate::types::ExtractedLedger;
use crate::xdr_limits;

/// Extract structured ledger data from a LedgerCloseMeta.
pub fn extract_ledger(meta: &LedgerCloseMeta) -> Result<ExtractedLedger, ParseError> {
    let header_entry = ledger_header_entry(meta);
    let header = &header_entry.header;

    let sequence = header.ledger_seq;
    let limits = xdr_limits::serialization_limits();

    let hash = header_entry
        .to_xdr(limits)
        .map(|xdr| hex::encode(Sha256::digest(&xdr)))
        .map_err(|e| ParseError {
            kind: ParseErrorKind::XdrSerializationFailed,
            message: format!("failed to serialize ledger header for hashing: {e}"),
            context: Some(ErrorContext {
                ledger_sequence: Some(sequence),
                transaction_index: None,
                transaction_hash: None,
                field: Some("hash".to_string()),
            }),
        })?;

    let closed_at = header.scp_value.close_time.0 as i64; // safe: Unix seconds fit i64
    let protocol_version = header.ledger_version;
    let base_fee = header.base_fee;
    let transaction_count: u32 = tx_count(meta).try_into().unwrap_or(u32::MAX);

    Ok(ExtractedLedger {
        sequence,
        hash,
        closed_at,
        protocol_version,
        transaction_count,
        base_fee,
    })
}

/// Get the ledger header entry from any LedgerCloseMeta variant.
fn ledger_header_entry(meta: &LedgerCloseMeta) -> &LedgerHeaderHistoryEntry {
    match meta {
        LedgerCloseMeta::V0(v) => &v.ledger_header,
        LedgerCloseMeta::V1(v) => &v.ledger_header,
        LedgerCloseMeta::V2(v) => &v.ledger_header,
    }
}

/// Get the transaction count from the processing results.
fn tx_count(meta: &LedgerCloseMeta) -> usize {
    match meta {
        LedgerCloseMeta::V0(v) => v.tx_processing.len(),
        LedgerCloseMeta::V1(v) => v.tx_processing.len(),
        LedgerCloseMeta::V2(v) => v.tx_processing.len(),
    }
}
