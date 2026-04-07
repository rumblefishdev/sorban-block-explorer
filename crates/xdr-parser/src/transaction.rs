//! Transaction extraction from LedgerCloseMeta.
//!
//! For each transaction in a ledger, extracts structured fields and retains
//! raw XDR payloads. Malformed transactions produce partial records with
//! `parse_error = true` — they are never dropped.

use base64::Engine;
use stellar_xdr::curr::*;
use tracing::warn;

use crate::envelope::{self, extract_envelopes, inner_transaction};
use crate::memo;
use crate::types::ExtractedTransaction;
use crate::xdr_limits;

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
    let limits = xdr_limits::serialization_limits();

    let tx_infos = collect_tx_infos(meta);
    let mut transactions = Vec::with_capacity(tx_infos.len());

    for (i, info) in tx_infos.iter().enumerate() {
        let tx = extract_single_transaction(
            info,
            envelopes.get(i),
            ledger_sequence,
            closed_at,
            i,
            &limits,
        );
        transactions.push(tx);
    }

    transactions
}

/// Unified transaction info extracted from V0/V1/V2 processing results.
struct TxInfo<'a> {
    hash: [u8; 32],
    fee_charged: i64,
    result: &'a TransactionResult,
    meta: &'a TransactionMeta,
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
                meta: &p.tx_apply_processing,
            })
            .collect(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .iter()
            .map(|p| TxInfo {
                hash: p.result.transaction_hash.0,
                fee_charged: p.result.result.fee_charged,
                result: &p.result.result,
                meta: &p.tx_apply_processing,
            })
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .iter()
            .map(|p| TxInfo {
                hash: p.result.transaction_hash.0,
                fee_charged: p.result.result.fee_charged,
                result: &p.result.result,
                meta: &p.tx_apply_processing,
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
    limits: &Limits,
) -> ExtractedTransaction {
    // Hash from TransactionResultPair — authoritative, avoids needing network_id.
    let hash = hex::encode(info.hash);
    let fee_charged = info.fee_charged;
    let successful = is_successful(&info.result.result);
    let result_code = info.result.result.name().to_string();

    let result_xdr = encode_xdr(info.result, limits, ledger_sequence, tx_index);
    let result_meta_xdr = encode_xdr_opt(info.meta, limits, ledger_sequence, tx_index);

    let (source_account, envelope_xdr, memo_type, memo_value) = match envelope {
        Some(env) => {
            let source = envelope::envelope_source(env);
            let env_xdr = encode_xdr(env, limits, ledger_sequence, tx_index);
            let inner = inner_transaction(env);
            let (mt, mv) = memo::extract_memo(inner.memo());
            (source, env_xdr, mt, mv)
        }
        None => {
            warn!(
                ledger_sequence,
                tx_index, "envelope missing for transaction — parse_error"
            );
            (String::new(), String::new(), None, None)
        }
    };

    let parse_error = envelope.is_none() || envelope_xdr.is_empty() || result_xdr.is_empty();

    ExtractedTransaction {
        hash,
        ledger_sequence,
        source_account,
        fee_charged,
        successful,
        result_code,
        envelope_xdr,
        result_xdr,
        result_meta_xdr,
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

fn encode_xdr<T: WriteXdr>(value: &T, limits: &Limits, ledger: u32, tx_idx: usize) -> String {
    match value.to_xdr(limits.clone()) {
        Ok(bytes) => base64::engine::general_purpose::STANDARD.encode(&bytes),
        Err(e) => {
            warn!(ledger, tx_idx, "XDR serialization failed: {e}");
            String::new()
        }
    }
}

fn encode_xdr_opt<T: WriteXdr>(
    value: &T,
    limits: &Limits,
    ledger: u32,
    tx_idx: usize,
) -> Option<String> {
    match value.to_xdr(limits.clone()) {
        Ok(bytes) => Some(base64::engine::general_purpose::STANDARD.encode(&bytes)),
        Err(e) => {
            warn!(ledger, tx_idx, "XDR serialization failed (nullable): {e}");
            None
        }
    }
}
