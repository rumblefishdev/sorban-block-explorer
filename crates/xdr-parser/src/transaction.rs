//! Transaction extraction from LedgerCloseMeta.
//!
//! For each transaction in a ledger, extracts structured fields.
//! Malformed transactions produce partial records with
//! `parse_error = true` — they are never dropped.

use stellar_xdr::curr::*;
use tracing::warn;

use crate::envelope::{self, extract_envelopes, inner_transaction};
use crate::memo;
use crate::types::ExtractedTransaction;

/// Extract all transactions from a LedgerCloseMeta.
///
/// Returns one `ExtractedTransaction` per transaction in the ledger.
/// Malformed transactions produce partial records with `parse_error = true`.
pub fn extract_transactions(
    meta: &LedgerCloseMeta,
    ledger_sequence: u32,
    closed_at: i64,
) -> Vec<ExtractedTransaction> {
    let envelopes = extract_envelopes(meta);

    let tx_infos = collect_tx_infos(meta);
    let mut transactions = Vec::with_capacity(tx_infos.len());

    for (i, info) in tx_infos.iter().enumerate() {
        let tx = extract_single_transaction(info, envelopes.get(i), ledger_sequence, closed_at, i);
        transactions.push(tx);
    }

    transactions
}

/// Unified transaction info extracted from V0/V1/V2 processing results.
struct TxInfo<'a> {
    hash: [u8; 32],
    fee_charged: i64,
    result: &'a TransactionResult,
}

/// Collect unified TxInfo from any LedgerCloseMeta variant.
fn collect_tx_infos(meta: &LedgerCloseMeta) -> Vec<TxInfo<'_>> {
    match meta {
        LedgerCloseMeta::V0(v) => v
            .tx_processing
            .iter()
            .map(|p| TxInfo {
                hash: p.result.transaction_hash.0,
                fee_charged: p.result.result.fee_charged,
                result: &p.result.result,
            })
            .collect(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .iter()
            .map(|p| TxInfo {
                hash: p.result.transaction_hash.0,
                fee_charged: p.result.result.fee_charged,
                result: &p.result.result,
            })
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .iter()
            .map(|p| TxInfo {
                hash: p.result.transaction_hash.0,
                fee_charged: p.result.result.fee_charged,
                result: &p.result.result,
            })
            .collect(),
    }
}

/// Extract a single transaction, producing a partial record on error.
fn extract_single_transaction(
    info: &TxInfo<'_>,
    envelope: Option<&TransactionEnvelope>,
    ledger_sequence: u32,
    closed_at: i64,
    tx_index: usize,
) -> ExtractedTransaction {
    // Hash from TransactionResultPair — authoritative, avoids needing network_id.
    let hash = hex::encode(info.hash);
    let fee_charged = info.fee_charged;
    let successful = is_successful(&info.result.result);
    let result_code = info.result.result.name().to_string();

    let (source_account, memo_type, memo_value, parse_error) = match envelope {
        Some(env) => {
            let source = envelope::envelope_source(env);
            let inner = inner_transaction(env);
            let (mt, mv) = memo::extract_memo(inner.memo());
            (source, mt, mv, false)
        }
        None => {
            warn!(
                ledger_sequence,
                tx_index, "envelope missing for transaction — parse_error"
            );
            (String::new(), None, None, true)
        }
    };

    ExtractedTransaction {
        hash,
        ledger_sequence,
        source_account,
        fee_charged,
        successful,
        result_code,
        memo_type,
        memo: memo_value,
        created_at: closed_at,
        operation_tree: None,
        parse_error,
    }
}

/// Check if a transaction result indicates success.
fn is_successful(result: &TransactionResultResult) -> bool {
    matches!(
        result,
        TransactionResultResult::TxSuccess(_) | TransactionResultResult::TxFeeBumpInnerSuccess(_)
    )
}
