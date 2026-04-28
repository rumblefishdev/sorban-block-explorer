//! Transaction envelope extraction from LedgerCloseMeta variants.
//!
//! Handles V0 (pre-protocol-20), V1 (generalized tx sets), and V2 (protocol 25+)
//! LedgerCloseMeta formats. Also handles V0 (classic) and V1 (parallel Soroban)
//! transaction phases within generalized tx sets.
//!
//! ## Apply-order alignment
//!
//! Per the Stellar protocol, transactions inside `tx_set` are sorted by hash
//! (deterministic consensus, see CAP-0063 for the parallel-Soroban phase),
//! while `tx_processing` is sorted in apply order (per the `Stellar-ledger.x`
//! comment "transactions are sorted in apply order here"). The two orders
//! do not match — empirically, 0/256 transactions aligned on the audited
//! mainnet ledger 62016099. Pairing by index would assign every tx the
//! wrong envelope, corrupting `source_id`, `operation_count`, `has_soroban`,
//! and any heavy-fields endpoint that joins envelope ↔ tx by position.
//!
//! `extract_envelopes` therefore returns envelopes aligned 1:1 to
//! `tx_processing`, matched by `SHA256(TransactionSignaturePayload)`.

use std::collections::HashMap;

use sha2::{Digest, Sha256};
use stellar_xdr::curr::*;
use tracing::warn;

/// Extract envelopes in **apply order**, aligned 1:1 with `tx_processing`.
///
/// Returned `Vec` length equals `tx_processing.len()`. Slot `i` carries the
/// envelope whose hash matches `tx_processing[i].result.transaction_hash`,
/// or `None` if no matching envelope exists in `tx_set` (corrupt
/// LedgerCloseMeta — never expected in well-formed data).
pub fn extract_envelopes(
    meta: &LedgerCloseMeta,
    network_id: &[u8; 32],
) -> Vec<Option<TransactionEnvelope>> {
    let raw = tx_set_envelopes(meta);
    let by_hash: HashMap<[u8; 32], TransactionEnvelope> = raw
        .into_iter()
        .map(|env| (tx_envelope_hash(&env, network_id), env))
        .collect();

    align_envelopes(by_hash, tx_processing_hashes(meta))
}

/// Pure helper: pull envelopes out of `by_hash` in the order given by
/// `target_hashes`. `remove` (rather than `get` + `clone`) avoids holding
/// duplicate copies of every envelope while we walk the apply order, and
/// drops each envelope from the map exactly when it lands in the output.
///
/// Misses are aggregated into a single warning per ledger with a sample
/// of up to 5 hex hashes so that a corrupt LedgerCloseMeta carrying many
/// missing envelopes does not flood the logs with one entry per tx.
fn align_envelopes(
    mut by_hash: HashMap<[u8; 32], TransactionEnvelope>,
    target_hashes: Vec<[u8; 32]>,
) -> Vec<Option<TransactionEnvelope>> {
    let mut envelopes = Vec::with_capacity(target_hashes.len());
    let mut missing_sample: Vec<[u8; 32]> = Vec::new();
    let mut missing_count: usize = 0;

    for h in &target_hashes {
        match by_hash.remove(h) {
            Some(env) => envelopes.push(Some(env)),
            None => {
                missing_count += 1;
                if missing_sample.len() < 5 {
                    missing_sample.push(*h);
                }
                envelopes.push(None);
            }
        }
    }

    if missing_count > 0 {
        let sample: Vec<String> = missing_sample.iter().map(hex::encode).collect();
        warn!(
            tx_processing_count = target_hashes.len(),
            missing_count,
            missing_sample = ?sample,
            "tx_processing hashes absent from tx_set — corrupt LedgerCloseMeta"
        );
    }

    envelopes
}

/// Compute the canonical Stellar transaction hash:
/// `SHA256(TransactionSignaturePayload(network_id, tagged_transaction))`.
///
/// V0 envelopes are promoted to V1 before hashing (matches stellar-core).
/// The result equals `TransactionResultPair.transaction_hash` for the same
/// envelope after apply.
pub fn tx_envelope_hash(env: &TransactionEnvelope, network_id: &[u8; 32]) -> [u8; 32] {
    let tagged = match env {
        TransactionEnvelope::TxV0(v0) => {
            let promoted = Transaction {
                source_account: MuxedAccount::from(&v0.tx.source_account_ed25519),
                fee: v0.tx.fee,
                seq_num: v0.tx.seq_num.clone(),
                cond: v0.tx.time_bounds.clone().into(),
                memo: v0.tx.memo.clone(),
                operations: v0.tx.operations.clone(),
                ext: v0.tx.ext.clone().into(),
            };
            TransactionSignaturePayloadTaggedTransaction::Tx(promoted)
        }
        TransactionEnvelope::Tx(v1) => {
            TransactionSignaturePayloadTaggedTransaction::Tx(v1.tx.clone())
        }
        TransactionEnvelope::TxFeeBump(fb) => {
            TransactionSignaturePayloadTaggedTransaction::TxFeeBump(fb.tx.clone())
        }
    };
    hash_tagged_transaction(tagged, network_id)
}

/// Compute the **inner** transaction hash for a fee-bump envelope.
///
/// Returns `Some(hash)` only for `TxFeeBump`; non-fee-bump envelopes return
/// `None` (the outer hash from `tx_envelope_hash` already IS the principal
/// hash for those). The inner hash is what Horizon reports as
/// `inner_transaction.hash` and what the inner-tx's sequence number was
/// signed against.
///
/// Per protocol the inner tx is a `TransactionV1` wrapped in
/// `FeeBumpTransactionInnerTx::Tx`. We re-tag it as
/// `TaggedTransaction::Tx(inner)` and hash against the same network_id.
pub fn inner_tx_hash(env: &TransactionEnvelope, network_id: &[u8; 32]) -> Option<[u8; 32]> {
    let TransactionEnvelope::TxFeeBump(fb) = env else {
        return None;
    };
    let FeeBumpTransactionInnerTx::Tx(inner) = &fb.tx.inner_tx;
    let tagged = TransactionSignaturePayloadTaggedTransaction::Tx(inner.tx.clone());
    Some(hash_tagged_transaction(tagged, network_id))
}

fn hash_tagged_transaction(
    tagged: TransactionSignaturePayloadTaggedTransaction,
    network_id: &[u8; 32],
) -> [u8; 32] {
    let payload = TransactionSignaturePayload {
        network_id: Hash(*network_id),
        tagged_transaction: tagged,
    };
    let bytes = payload
        .to_xdr(Limits::none())
        .expect("TransactionSignaturePayload encode is infallible for well-formed input");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hasher.finalize().into()
}

fn tx_set_envelopes(meta: &LedgerCloseMeta) -> Vec<TransactionEnvelope> {
    let mut envelopes = Vec::new();
    match meta {
        LedgerCloseMeta::V0(v) => {
            for env in v.tx_set.txs.iter() {
                envelopes.push(env.clone());
            }
        }
        LedgerCloseMeta::V1(v) => {
            let GeneralizedTransactionSet::V1(ts) = &v.tx_set;
            for phase in ts.phases.iter() {
                collect_phase_envelopes(phase, &mut envelopes);
            }
        }
        LedgerCloseMeta::V2(v) => {
            let GeneralizedTransactionSet::V1(ts) = &v.tx_set;
            for phase in ts.phases.iter() {
                collect_phase_envelopes(phase, &mut envelopes);
            }
        }
    }
    envelopes
}

fn tx_processing_hashes(meta: &LedgerCloseMeta) -> Vec<[u8; 32]> {
    match meta {
        LedgerCloseMeta::V0(v) => v
            .tx_processing
            .iter()
            .map(|p| p.result.transaction_hash.0)
            .collect(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .iter()
            .map(|p| p.result.transaction_hash.0)
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .iter()
            .map(|p| p.result.transaction_hash.0)
            .collect(),
    }
}

/// Collect envelopes from a transaction phase (V0 classic or V1 parallel Soroban).
fn collect_phase_envelopes(phase: &TransactionPhase, out: &mut Vec<TransactionEnvelope>) {
    match phase {
        TransactionPhase::V0(components) => {
            for comp in components.iter() {
                let TxSetComponent::TxsetCompTxsMaybeDiscountedFee(txs) = comp;
                for env in txs.txs.iter() {
                    out.push(env.clone());
                }
            }
        }
        TransactionPhase::V1(parallel) => {
            for stage in parallel.execution_stages.iter() {
                for cluster in stage.0.iter() {
                    for env in cluster.0.iter() {
                        out.push(env.clone());
                    }
                }
            }
        }
    }
}

/// Extract the source account address from a transaction envelope.
///
/// For fee-bump envelopes this returns the **inner** transaction's source
/// account, not `fee_source` — the fee-bump payer is metadata, not the
/// principal whose sequence is consumed and whose ops execute.
pub fn envelope_source(env: &TransactionEnvelope) -> String {
    inner_transaction(env).source_account()
}

/// Get a reference to the inner transaction for memo extraction.
/// For fee-bump transactions, returns the inner transaction.
pub fn inner_transaction(env: &TransactionEnvelope) -> InnerTxRef<'_> {
    match env {
        TransactionEnvelope::TxV0(v0) => InnerTxRef::V0(&v0.tx),
        TransactionEnvelope::Tx(v1) => InnerTxRef::V1(&v1.tx),
        TransactionEnvelope::TxFeeBump(fb) => {
            let FeeBumpTransactionInnerTx::Tx(inner) = &fb.tx.inner_tx;
            InnerTxRef::V1(&inner.tx)
        }
    }
}

/// Reference to the inner transaction, regardless of envelope version.
pub enum InnerTxRef<'a> {
    V0(&'a TransactionV0),
    V1(&'a Transaction),
}

impl<'a> InnerTxRef<'a> {
    pub fn memo(&self) -> &Memo {
        match self {
            InnerTxRef::V0(tx) => &tx.memo,
            InnerTxRef::V1(tx) => &tx.memo,
        }
    }

    pub fn source_account(&self) -> String {
        match self {
            InnerTxRef::V0(tx) => MuxedAccount::from(&tx.source_account_ed25519).to_string(),
            InnerTxRef::V1(tx) => tx.source_account.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Coverage for `envelope_source()` across all `TransactionEnvelope`
    //! variants. The fee-bump regression test guards bug 0168: the indexer
    //! used to write `fee_source` (the fee-bump payer) into
    //! `transactions.source_id`, masking the principal account whose
    //! sequence number is consumed and whose ops execute.
    use super::*;

    fn ed25519_strkey(payload: &[u8; 32]) -> String {
        MuxedAccount::Ed25519(Uint256(*payload)).to_string()
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

    fn v0_envelope(source_payload: [u8; 32]) -> TransactionEnvelope {
        TransactionEnvelope::TxV0(TransactionV0Envelope {
            tx: TransactionV0 {
                source_account_ed25519: Uint256(source_payload),
                fee: 100,
                seq_num: SequenceNumber(1),
                time_bounds: None,
                memo: Memo::None,
                operations: VecM::default(),
                ext: TransactionV0Ext::V0,
            },
            signatures: VecM::default(),
        })
    }

    fn v1_envelope(source_payload: [u8; 32]) -> TransactionEnvelope {
        TransactionEnvelope::Tx(TransactionV1Envelope {
            tx: empty_v1_tx(source_payload),
            signatures: VecM::default(),
        })
    }

    fn fee_bump_envelope(
        fee_source_payload: [u8; 32],
        inner_source_payload: [u8; 32],
    ) -> TransactionEnvelope {
        TransactionEnvelope::TxFeeBump(FeeBumpTransactionEnvelope {
            tx: FeeBumpTransaction {
                fee_source: MuxedAccount::Ed25519(Uint256(fee_source_payload)),
                fee: 200,
                inner_tx: FeeBumpTransactionInnerTx::Tx(TransactionV1Envelope {
                    tx: empty_v1_tx(inner_source_payload),
                    signatures: VecM::default(),
                }),
                ext: FeeBumpTransactionExt::V0,
            },
            signatures: VecM::default(),
        })
    }

    #[test]
    fn envelope_source_v0_unwraps_bare_ed25519_payload() {
        let env = v0_envelope([0xAA; 32]);
        assert_eq!(envelope_source(&env), ed25519_strkey(&[0xAA; 32]));
    }

    #[test]
    fn envelope_source_v1_returns_tx_source_account() {
        let env = v1_envelope([0xBB; 32]);
        assert_eq!(envelope_source(&env), ed25519_strkey(&[0xBB; 32]));
    }

    #[test]
    fn envelope_source_fee_bump_returns_inner_source_not_fee_source() {
        // Bug 0168 regression: indexer used to write the fee-bump payer
        // (fee_source) into transactions.source_id. Must return the inner
        // transaction's source instead.
        let fee_source = [0xCC; 32];
        let inner_source = [0xDD; 32];
        let env = fee_bump_envelope(fee_source, inner_source);

        let got = envelope_source(&env);
        assert_eq!(got, ed25519_strkey(&inner_source));
        assert_ne!(got, ed25519_strkey(&fee_source));
    }

    #[test]
    fn inner_tx_hash_returns_none_for_non_fee_bump_envelopes() {
        // Bug 0169 regression: only fee-bump envelopes carry an inner hash;
        // V0 / V1 envelopes' principal hash already comes from
        // tx_envelope_hash, so inner_tx_hash MUST return None there.
        assert!(inner_tx_hash(&v0_envelope([0xAA; 32]), &NET_ID).is_none());
        assert!(inner_tx_hash(&v1_envelope([0xBB; 32]), &NET_ID).is_none());
    }

    #[test]
    fn inner_tx_hash_for_fee_bump_differs_from_outer_and_matches_inner_v1_hash() {
        // The inner hash is what Horizon reports as `inner_transaction.hash`.
        // Construct a fee-bump wrapping a known V1 inner; the inner hash
        // must equal `tx_envelope_hash` of the standalone V1 envelope and
        // must NOT equal the outer (TxFeeBump) hash.
        let fee_source = [0x11; 32];
        let inner_source = [0x22; 32];
        let fb_env = fee_bump_envelope(fee_source, inner_source);
        let inner_v1_env = v1_envelope(inner_source);

        let outer = tx_envelope_hash(&fb_env, &NET_ID);
        let inner_via_helper = inner_tx_hash(&fb_env, &NET_ID).expect("fee-bump has inner");
        let inner_via_v1 = tx_envelope_hash(&inner_v1_env, &NET_ID);

        assert_eq!(inner_via_helper, inner_via_v1);
        assert_ne!(inner_via_helper, outer);
    }

    // --- alignment: synthetic in-memory coverage of `align_envelopes` ---
    //
    // The `tests/envelope_apply_order.rs` integration test exercises the
    // same logic against a real LedgerCloseMeta when one is available
    // (`.temp/` fixture or `XDR_FIXTURE` env). These unit tests run
    // unconditionally in `cargo test` so the alignment behavior is
    // validated even without a fixture.

    const NET_ID: [u8; 32] = [0u8; 32];

    fn three_distinct_v1_envs() -> [TransactionEnvelope; 3] {
        [
            v1_envelope([0x11; 32]),
            v1_envelope([0x22; 32]),
            v1_envelope([0x33; 32]),
        ]
    }

    fn build_by_hash(envs: &[TransactionEnvelope]) -> HashMap<[u8; 32], TransactionEnvelope> {
        envs.iter()
            .cloned()
            .map(|e| (tx_envelope_hash(&e, &NET_ID), e))
            .collect()
    }

    #[test]
    fn align_envelopes_reorders_by_target_hashes() {
        let envs = three_distinct_v1_envs();
        let hashes: Vec<[u8; 32]> = envs.iter().map(|e| tx_envelope_hash(e, &NET_ID)).collect();
        // Apply order = reverse of tx_set order — a guaranteed mismatch
        // for index-based pairing.
        let target = vec![hashes[2], hashes[0], hashes[1]];

        let aligned = align_envelopes(build_by_hash(&envs), target);

        let aligned_hashes: Vec<[u8; 32]> = aligned
            .iter()
            .map(|o| tx_envelope_hash(o.as_ref().unwrap(), &NET_ID))
            .collect();
        assert_eq!(aligned_hashes, vec![hashes[2], hashes[0], hashes[1]]);
    }

    #[test]
    fn align_envelopes_returns_none_for_unmatched_hash_and_preserves_indices() {
        let envs = three_distinct_v1_envs();
        let hashes: Vec<[u8; 32]> = envs.iter().map(|e| tx_envelope_hash(e, &NET_ID)).collect();
        // Middle slot points at a hash that doesn't exist in tx_set (corrupt
        // ledger). Index alignment with tx_processing must be preserved —
        // slot 1 = None, slots 0 and 2 still resolve.
        let bogus = [0x99; 32];
        let target = vec![hashes[0], bogus, hashes[2]];

        let aligned = align_envelopes(build_by_hash(&envs), target);

        assert!(aligned[0].is_some());
        assert!(aligned[1].is_none());
        assert!(aligned[2].is_some());
        assert_eq!(
            tx_envelope_hash(aligned[0].as_ref().unwrap(), &NET_ID),
            hashes[0]
        );
        assert_eq!(
            tx_envelope_hash(aligned[2].as_ref().unwrap(), &NET_ID),
            hashes[2]
        );
    }

    #[test]
    fn align_envelopes_empty_target_returns_empty_vec() {
        let envs = three_distinct_v1_envs();
        let aligned = align_envelopes(build_by_hash(&envs), Vec::new());
        assert!(aligned.is_empty());
    }
}
