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

    tx_processing_hashes(meta)
        .into_iter()
        .map(|h| {
            by_hash.get(&h).cloned().or_else(|| {
                warn!(
                    tx_hash = %hex::encode(h),
                    "tx_processing hash absent from tx_set — corrupt LedgerCloseMeta"
                );
                None
            })
        })
        .collect()
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
}
