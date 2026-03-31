//! Transaction envelope extraction from LedgerCloseMeta variants.
//!
//! Handles V0 (pre-protocol-20), V1 (generalized tx sets), and V2 (protocol 25+)
//! LedgerCloseMeta formats. Also handles V0 (classic) and V1 (parallel Soroban)
//! transaction phases within generalized tx sets.

use stellar_xdr::curr::*;

/// Extract all transaction envelopes from a LedgerCloseMeta.
pub fn extract_envelopes(meta: &LedgerCloseMeta) -> Vec<TransactionEnvelope> {
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
pub fn envelope_source(env: &TransactionEnvelope) -> String {
    match env {
        TransactionEnvelope::TxV0(v0) => {
            MuxedAccount::from(&v0.tx.source_account_ed25519).to_string()
        }
        TransactionEnvelope::Tx(v1) => v1.tx.source_account.to_string(),
        TransactionEnvelope::TxFeeBump(fb) => fb.tx.fee_source.to_string(),
    }
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
}
