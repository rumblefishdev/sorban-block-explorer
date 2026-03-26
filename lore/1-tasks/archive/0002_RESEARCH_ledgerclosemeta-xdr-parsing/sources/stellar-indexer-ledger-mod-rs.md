---
source: rumblefishdev/stellar-indexer (private repo)
file: src/ledger/mod.rs
retrieved: 2026-03-26
note: Copied from local checkout at /Users/stkrolikiewicz/Developer/RumbleFish/stellar-indexer
---

# stellar-indexer: src/ledger/mod.rs

LedgerCloseMeta parsing and envelope extraction in Rust. Key patterns:

- V0/V1/V2 LedgerCloseMeta version handling
- TransactionPhase::V0 (classic) and V1 (parallel) envelope extraction
- TX result and meta lookup by hash

```rust
use stellar_xdr::curr::{
    GeneralizedTransactionSet, LedgerCloseMeta, TransactionEnvelope, TransactionMeta,
    TransactionPhase, TransactionResult, TxSetComponent,
};

pub fn extract_ledger_info(meta: &LedgerCloseMeta) -> (u32, u64, usize) {
    match meta {
        LedgerCloseMeta::V0(v) => {
            let seq = v.ledger_header.header.ledger_seq;
            let close_time: u64 = v.ledger_header.header.scp_value.close_time.clone().into();
            let tx_count = v.tx_processing.len();
            (seq, close_time, tx_count)
        }
        LedgerCloseMeta::V1(v) => {
            let seq = v.ledger_header.header.ledger_seq;
            let close_time: u64 = v.ledger_header.header.scp_value.close_time.clone().into();
            let tx_count = v.tx_processing.len();
            (seq, close_time, tx_count)
        }
        LedgerCloseMeta::V2(v) => {
            let seq = v.ledger_header.header.ledger_seq;
            let close_time: u64 = v.ledger_header.header.scp_value.close_time.clone().into();
            let tx_count = v.tx_processing.len();
            (seq, close_time, tx_count)
        }
    }
}

fn for_envelopes_in_phase<F>(phase: &TransactionPhase, f: &mut F)
where
    F: FnMut(&TransactionEnvelope),
{
    match phase {
        TransactionPhase::V0(components) => {
            for comp in components.iter() {
                let TxSetComponent::TxsetCompTxsMaybeDiscountedFee(txs_comp) = comp;
                for env in txs_comp.txs.iter() {
                    f(env);
                }
            }
        }
        TransactionPhase::V1(parallel) => {
            for stage in parallel.execution_stages.iter() {
                for cluster in stage.0.iter() {
                    for env in cluster.0.iter() {
                        f(env);
                    }
                }
            }
        }
    }
}

pub fn for_each_envelope<F>(meta: &LedgerCloseMeta, mut f: F)
where
    F: FnMut(&TransactionEnvelope),
{
    match meta {
        LedgerCloseMeta::V0(v) => {
            for env in v.tx_set.txs.iter() {
                f(env);
            }
        }
        LedgerCloseMeta::V1(v) => {
            let GeneralizedTransactionSet::V1(ts1) = &v.tx_set;
            for phase in ts1.phases.iter() {
                for_envelopes_in_phase(phase, &mut f);
            }
        }
        LedgerCloseMeta::V2(v) => {
            let GeneralizedTransactionSet::V1(ts1) = &v.tx_set;
            for phase in ts1.phases.iter() {
                for_envelopes_in_phase(phase, &mut f);
            }
        }
    }
}

pub fn find_tx_result_by_hash<'a>(
    meta: &'a LedgerCloseMeta,
    tx_hash: &[u8; 32],
) -> Option<&'a TransactionResult> {
    match meta {
        LedgerCloseMeta::V0(v) => v.tx_processing.iter().find_map(|m| {
            if m.result.transaction_hash.0 == *tx_hash {
                Some(&m.result.result)
            } else {
                None
            }
        }),
        LedgerCloseMeta::V1(v) => v.tx_processing.iter().find_map(|m| {
            if m.result.transaction_hash.0 == *tx_hash {
                Some(&m.result.result)
            } else {
                None
            }
        }),
        LedgerCloseMeta::V2(v) => v.tx_processing.iter().find_map(|m| {
            if m.result.transaction_hash.0 == *tx_hash {
                Some(&m.result.result)
            } else {
                None
            }
        }),
    }
}
```
