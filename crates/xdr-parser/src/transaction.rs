//! Transaction extraction from LedgerCloseMeta.
//!
//! For each transaction in a ledger, extracts structured fields and retains
//! raw XDR payloads. Malformed transactions produce partial records with
//! `parse_error = true` — they are never dropped.

use base64::Engine;
use stellar_xdr::*;
use tracing::warn;

use crate::envelope::{self, extract_envelopes, inner_transaction, inner_tx_hash};
use crate::memo;
use crate::types::ExtractedTransaction;
use crate::xdr_limits;

/// Extract all transactions from a LedgerCloseMeta.
///
/// Returns one `ExtractedTransaction` per transaction in the ledger, in
/// **apply order** (matching `tx_processing`). Malformed transactions
/// produce partial records with `parse_error = true`.
///
/// `network_id` is required to align envelopes from `tx_set` (hash-sorted)
/// with `tx_processing` (apply order) — see `envelope::extract_envelopes`.
pub fn extract_transactions(
    meta: &LedgerCloseMeta,
    ledger_sequence: u32,
    closed_at: i64,
    network_id: &[u8; 32],
) -> Vec<ExtractedTransaction> {
    let envelopes = extract_envelopes(meta, network_id);
    let limits = xdr_limits::serialization_limits();

    let tx_infos = collect_tx_infos(meta);
    let mut transactions = Vec::with_capacity(tx_infos.len());

    for (i, info) in tx_infos.iter().enumerate() {
        let tx = extract_single_transaction(
            info,
            envelopes.get(i).and_then(Option::as_ref),
            ledger_sequence,
            closed_at,
            i,
            &limits,
            network_id,
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

/// Collect per-transaction `TransactionResult` references from any
/// LedgerCloseMeta variant. Index-aligned with `extract_envelopes` and the
/// `collect_tx_metas` helpers (same `tx_processing` order). Feed each entry
/// through `operation::tx_op_results` to reach the per-op results consumed
/// by `extract_operations` (task 0261).
pub fn collect_tx_results(meta: &LedgerCloseMeta) -> Vec<&TransactionResult> {
    match meta {
        LedgerCloseMeta::V0(v) => v.tx_processing.iter().map(|p| &p.result.result).collect(),
        LedgerCloseMeta::V1(v) => v.tx_processing.iter().map(|p| &p.result.result).collect(),
        LedgerCloseMeta::V2(v) => v.tx_processing.iter().map(|p| &p.result.result).collect(),
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
    network_id: &[u8; 32],
) -> ExtractedTransaction {
    // Hash from TransactionResultPair — authoritative, avoids needing network_id.
    let hash = hex::encode(info.hash);
    let fee_charged = info.fee_charged;
    let successful = is_successful(&info.result.result);
    let result_code = info.result.result.name().to_string();

    let result_xdr = encode_xdr(info.result, limits, ledger_sequence, tx_index);
    let result_meta_xdr = encode_xdr_opt(info.meta, limits, ledger_sequence, tx_index);

    let (
        source_account,
        source_muxed_id,
        fee_source,
        envelope_xdr,
        memo_type,
        memo_value,
        inner_tx_hash_hex,
    ) = match envelope {
        Some(env) => {
            let source = envelope::envelope_source(env);
            let fee_source = envelope::envelope_fee_source(env);
            let env_xdr = encode_xdr(env, limits, ledger_sequence, tx_index);
            let inner = inner_transaction(env);
            let source_muxed_id = inner.source_muxed_id();
            let (mt, mv) = memo::extract_memo(inner.memo());
            let inner_hash = inner_tx_hash(env, network_id).map(hex::encode);
            (
                source,
                source_muxed_id,
                fee_source,
                env_xdr,
                mt,
                mv,
                inner_hash,
            )
        }
        None => {
            warn!(
                ledger_sequence,
                tx_index, "envelope missing for transaction — parse_error"
            );
            (String::new(), None, None, String::new(), None, None, None)
        }
    };

    let parse_error = envelope.is_none() || envelope_xdr.is_empty() || result_xdr.is_empty();

    ExtractedTransaction {
        hash,
        inner_tx_hash: inner_tx_hash_hex,
        ledger_sequence,
        source_account,
        source_muxed_id,
        fee_source,
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
        ledger_deltas: crate::ledger_balance_deltas(info.meta),
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

#[cfg(test)]
mod parse_error_tests {
    //! Coverage for the `parse_error = true` code path in
    //! `extract_single_transaction` (task 0190).
    //!
    //! Three reachable triggers per the body below:
    //!
    //! ```ignore
    //! let parse_error = envelope.is_none()
    //!                || envelope_xdr.is_empty()
    //!                || result_xdr.is_empty();
    //! ```
    //!
    //! Variant A — missing envelope (`envelope.is_none()`): build a
    //! synthetic `LedgerCloseMeta` whose `tx_set` contains one envelope
    //! while `tx_processing` carries two entries (the second with an
    //! unmatched hash). `extract_envelopes` aligns by hash → returns
    //! `[Some, None]`, so the second transaction lands with
    //! `envelope = None` and `parse_error = true`. Exercised through
    //! the public `extract_transactions` entry point.
    //!
    //! Variant B — encode failure (`envelope_xdr.is_empty()` and
    //! `result_xdr.is_empty()`): call the private
    //! `extract_single_transaction` directly with `Limits { depth: 0,
    //! len: 0 }` so `to_xdr(limits)` refuses both the envelope and the
    //! result blob. The envelope itself is fully formed — alignment
    //! would succeed in the wider pipeline — only the **retention**
    //! encoding flips.
    //!
    //! Regression sentinel — the same aligned fixture run through the
    //! public `extract_transactions` (default `serialization_limits`)
    //! MUST keep `parse_error = false`. A regression that silently
    //! tightened the default limits would otherwise pass Variant B and
    //! corrupt every real ledger.
    //!
    //! Tests live inside this module (rather than `tests/`) so they can
    //! reach the private `extract_single_transaction` and `TxInfo` types
    //! without expanding the crate's public surface for what is purely
    //! a test affordance.

    use super::*;
    use crate::envelope::tx_envelope_hash;
    use crate::{MAINNET_PASSPHRASE, network_id};
    use sha2::{Digest, Sha256};
    use stellar_xdr::{
        ExtensionPoint, Hash, LedgerCloseMetaV0, LedgerEntryChanges, LedgerHeader, LedgerHeaderExt,
        LedgerHeaderHistoryEntry, LedgerHeaderHistoryEntryExt, Memo, MuxedAccount, OperationResult,
        Preconditions, SequenceNumber, StellarValue, StellarValueExt, TimePoint, Transaction,
        TransactionEnvelope, TransactionExt, TransactionMetaV3, TransactionResultExt,
        TransactionResultMeta, TransactionResultPair, TransactionSet, TransactionV1Envelope,
        Uint256, VecM,
    };

    // ---- Synthetic-fixture builders --------------------------------------

    fn synthetic_ledger_header_entry(ledger_seq: u32) -> LedgerHeaderHistoryEntry {
        let header = LedgerHeader {
            ledger_version: 23,
            previous_ledger_hash: Hash([0; 32]),
            scp_value: StellarValue {
                tx_set_hash: Hash([0; 32]),
                close_time: TimePoint(1_700_000_000),
                upgrades: VecM::default(),
                ext: StellarValueExt::Basic,
            },
            tx_set_result_hash: Hash([0; 32]),
            bucket_list_hash: Hash([0; 32]),
            ledger_seq,
            total_coins: 0,
            fee_pool: 0,
            inflation_seq: 0,
            id_pool: 0,
            base_fee: 100,
            base_reserve: 0,
            max_tx_set_size: 0,
            skip_list: [Hash([0; 32]), Hash([0; 32]), Hash([0; 32]), Hash([0; 32])],
            ext: LedgerHeaderExt::V0,
        };
        LedgerHeaderHistoryEntry {
            hash: Hash([0; 32]),
            header,
            ext: LedgerHeaderHistoryEntryExt::V0,
        }
    }

    fn empty_v1_tx(source_payload: [u8; 32]) -> Transaction {
        Transaction {
            source_account: MuxedAccount::Ed25519(Uint256(source_payload)),
            fee: 100,
            seq_num: SequenceNumber(1),
            cond: Preconditions::None,
            memo: Memo::None,
            operations: VecM::default(),
            ext: TransactionExt::V0,
        }
    }

    fn v1_envelope(source_payload: [u8; 32]) -> TransactionEnvelope {
        TransactionEnvelope::Tx(TransactionV1Envelope {
            tx: empty_v1_tx(source_payload),
            signatures: VecM::default(),
        })
    }

    fn empty_tx_result(fee_charged: i64) -> TransactionResult {
        let op_results: VecM<OperationResult> = VecM::default();
        TransactionResult {
            fee_charged,
            result: TransactionResultResult::TxSuccess(op_results),
            ext: TransactionResultExt::V0,
        }
    }

    fn empty_tx_meta() -> TransactionMeta {
        TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        })
    }

    fn result_meta_for_hash(hash: [u8; 32], fee_charged: i64) -> TransactionResultMeta {
        TransactionResultMeta {
            result: TransactionResultPair {
                transaction_hash: Hash(hash),
                result: empty_tx_result(fee_charged),
            },
            fee_processing: LedgerEntryChanges::default(),
            tx_apply_processing: empty_tx_meta(),
        }
    }

    /// Deterministic sentinel hash that no real envelope in the fixture
    /// can produce. `align_envelopes` will fail to find it in
    /// `by_hash` and emit `None` for the corresponding slot.
    fn unmatched_hash() -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"task-0190 unmatched envelope hash sentinel");
        h.finalize().into()
    }

    // ---- Variant A — missing envelope -----------------------------------

    #[test]
    fn variant_a_missing_envelope_marks_parse_error_true_for_unaligned_slot() {
        let net_id = network_id(MAINNET_PASSPHRASE);

        let env_real = v1_envelope([0x11; 32]);
        let env_real_hash = tx_envelope_hash(&env_real, &net_id);

        let tx_set = TransactionSet {
            previous_ledger_hash: Hash([0; 32]),
            txs: vec![env_real].try_into().expect("1-elem VecM"),
        };

        let proc_aligned = result_meta_for_hash(env_real_hash, /*fee*/ 200);
        let proc_unaligned = result_meta_for_hash(unmatched_hash(), /*fee*/ 300);

        let meta = LedgerCloseMeta::V0(LedgerCloseMetaV0 {
            ledger_header: synthetic_ledger_header_entry(12_345),
            tx_set,
            tx_processing: vec![proc_aligned, proc_unaligned]
                .try_into()
                .expect("2-elem VecM"),
            upgrades_processing: VecM::default(),
            scp_info: VecM::default(),
        });

        let txs = extract_transactions(&meta, 12_345, 1_700_000_000, &net_id);

        assert_eq!(
            txs.len(),
            2,
            "one ExtractedTransaction per tx_processing entry"
        );

        // Slot 0 — aligned envelope; parser produces a fully-formed record.
        assert!(
            !txs[0].parse_error,
            "slot 0 aligned with a real envelope, must NOT carry parse_error: {:?}",
            txs[0]
        );
        assert!(
            !txs[0].envelope_xdr.is_empty(),
            "slot 0 envelope_xdr must be populated for the aligned envelope"
        );
        assert!(
            !txs[0].source_account.is_empty(),
            "slot 0 source_account must come from the aligned envelope"
        );

        // Slot 1 — the unaligned tx_processing entry; failure path the
        // test exists to lock down.
        assert!(
            txs[1].parse_error,
            "slot 1 has no aligned envelope → parse_error MUST be true: {:?}",
            txs[1]
        );
        assert!(txs[1].envelope_xdr.is_empty());
        assert!(txs[1].source_account.is_empty());
        assert!(txs[1].memo_type.is_none());
        assert!(txs[1].memo.is_none());
        assert!(txs[1].inner_tx_hash.is_none());

        // Hash + fee_charged come from `TransactionResultPair` (not the
        // envelope), so they survive the missing-envelope case. The
        // downstream contract from lore-0014 is "log, store raw, mark
        // parse_error, keep visible" — the row stays identifiable.
        assert_eq!(
            txs[1].hash,
            hex::encode(unmatched_hash()),
            "slot 1 hash must surface from TransactionResultPair even when envelope is missing"
        );
        assert_eq!(txs[1].fee_charged, 300);
        assert_eq!(txs[1].ledger_sequence, 12_345);
        assert_eq!(txs[1].created_at, 1_700_000_000);
    }

    // ---- Variant B — encode failure -------------------------------------

    /// Call `extract_single_transaction` directly with `Limits { 0, 0 }`
    /// so `to_xdr(limits)` refuses both the envelope and the result
    /// blob. The envelope is fully formed and aligned — only the
    /// retention encoding fails. Distinguishing trait from Variant A:
    /// `source_account` stays **populated** because it comes from
    /// envelope inspection, not from re-encoding.
    #[test]
    fn variant_b_encode_failure_marks_parse_error_true_even_with_aligned_envelope() {
        let net_id = network_id(MAINNET_PASSPHRASE);

        let env_real = v1_envelope([0x22; 32]);
        let env_real_hash = tx_envelope_hash(&env_real, &net_id);

        let result = empty_tx_result(/*fee*/ 400);
        let tx_meta = empty_tx_meta();
        let info = TxInfo {
            hash: env_real_hash,
            fee_charged: 400,
            result: &result,
            meta: &tx_meta,
        };

        // Zero-capacity limits — refuse any encoded byte.
        let zero_limits = Limits { depth: 0, len: 0 };

        let tx = extract_single_transaction(
            &info,
            Some(&env_real),
            /*ledger_sequence*/ 54_321,
            /*closed_at*/ 1_700_000_000,
            /*tx_index*/ 0,
            &zero_limits,
            &net_id,
        );

        assert!(
            tx.parse_error,
            "encode failure (empty envelope_xdr / result_xdr) MUST set parse_error: {tx:?}",
        );
        assert!(
            tx.envelope_xdr.is_empty(),
            "envelope_xdr must be empty when to_xdr(limits) fails"
        );
        assert!(
            tx.result_xdr.is_empty(),
            "result_xdr must be empty when to_xdr(limits) fails"
        );

        // Envelope inspection succeeded (tx_envelope_hash uses
        // `Limits::none()` internally — independent of caller-supplied
        // limits — and envelope_source reads the source field directly
        // without re-encoding). Variant A would have `source_account`
        // empty here; Variant B preserves it.
        assert!(
            !tx.source_account.is_empty(),
            "Variant B preserves source_account: envelope was inspected, only re-encoding \
             failed. Empty source_account would mean envelope alignment regressed."
        );

        assert_eq!(tx.hash, hex::encode(env_real_hash));
        assert_eq!(tx.fee_charged, 400);
        assert_eq!(tx.ledger_sequence, 54_321);
    }

    // ---- Regression sentinel --------------------------------------------

    /// Same aligned-envelope fixture run through `extract_transactions`
    /// (default `serialization_limits`). Without this sentinel, a
    /// regression that silently flipped the default limits to
    /// `{ depth: 0, len: 0 }` would pass Variant B (it expects tight
    /// limits) while corrupting every real ledger.
    #[test]
    fn default_limits_keep_parse_error_false_for_aligned_envelope() {
        let net_id = network_id(MAINNET_PASSPHRASE);

        let env_real = v1_envelope([0x33; 32]);
        let env_real_hash = tx_envelope_hash(&env_real, &net_id);

        let tx_set = TransactionSet {
            previous_ledger_hash: Hash([0; 32]),
            txs: vec![env_real].try_into().expect("1-elem VecM"),
        };
        let proc_aligned = result_meta_for_hash(env_real_hash, /*fee*/ 500);

        let meta = LedgerCloseMeta::V0(LedgerCloseMetaV0 {
            ledger_header: synthetic_ledger_header_entry(77_777),
            tx_set,
            tx_processing: vec![proc_aligned].try_into().expect("1-elem VecM"),
            upgrades_processing: VecM::default(),
            scp_info: VecM::default(),
        });

        let txs = extract_transactions(&meta, 77_777, 1_700_000_000, &net_id);

        assert_eq!(txs.len(), 1);
        assert!(
            !txs[0].parse_error,
            "default serialization_limits must NOT trip parse_error on a well-formed \
             aligned envelope (regression sentinel for Variant B)"
        );
        assert!(!txs[0].envelope_xdr.is_empty());
        assert!(!txs[0].result_xdr.is_empty());
    }
}
