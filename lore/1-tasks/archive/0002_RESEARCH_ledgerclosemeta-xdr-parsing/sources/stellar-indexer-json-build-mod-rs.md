---
source: rumblefishdev/stellar-indexer (private repo)
file: src/json_build/mod.rs
retrieved: 2026-03-26
note: Copied from local checkout. TX JSON builder with envelope parsing and Soroban resource extraction.
---

# stellar-indexer: src/json_build/mod.rs

Transaction JSON builder. Key patterns:

- Envelope type dispatch: TxV0, Tx (V1), TxFeeBump
- Source account and fee extraction from each envelope type
- Soroban resource extraction from transaction ext (instructions, disk read/write bytes)
- InvokeHostFunction operation details: contract address, method, typed parameters
- TX hash comes from `env.hash(network_id)` — built-in Rust SDK method

```rust
use serde_json::json;
use stellar_xdr::curr::{
    FeeBumpTransactionInnerTx, HostFunction, InvokeHostFunctionOp, Limits, MuxedAccount, Operation,
    OperationBody, TransactionEnvelope, TransactionExt, TransactionMeta, TransactionResult,
    TransactionResultResult,
};
use stellar_xdr::curr::WriteXdr;

use crate::events::events_from_tx_meta_to_json;
use crate::scval::{format_muxed_account, scval_to_typed_json_value};

fn soroban_resources_from_envelope(env: &TransactionEnvelope) -> Option<serde_json::Value> {
    let tx = match env {
        TransactionEnvelope::Tx(v1) => &v1.tx,
        TransactionEnvelope::TxFeeBump(fb) => {
            let FeeBumpTransactionInnerTx::Tx(inner) = &fb.tx.inner_tx;
            &inner.tx
        }
        TransactionEnvelope::TxV0(_) => return None,
    };
    let TransactionExt::V1(soroban_data) = &tx.ext else {
        return None;
    };
    Some(json!({
        "instructions": soroban_data.resources.instructions,
        "disk_read_bytes": soroban_data.resources.disk_read_bytes,
        "write_bytes": soroban_data.resources.write_bytes,
    }))
}

fn envelope_sender_and_ops<'a>(
    env: &'a TransactionEnvelope,
) -> (String, u64, Vec<&'a Operation>) {
    match env {
        TransactionEnvelope::TxV0(v0) => (
            format_muxed_account(&MuxedAccount::from(&v0.tx.source_account_ed25519)),
            v0.tx.fee as u64,
            v0.tx.operations.iter().collect::<Vec<_>>(),
        ),
        TransactionEnvelope::Tx(v1) => (
            format_muxed_account(&v1.tx.source_account),
            v1.tx.fee as u64,
            v1.tx.operations.iter().collect::<Vec<_>>(),
        ),
        TransactionEnvelope::TxFeeBump(fb) => {
            let FeeBumpTransactionInnerTx::Tx(inner) = &fb.tx.inner_tx;
            (
                format_muxed_account(&inner.tx.source_account),
                inner.tx.fee as u64,
                inner.tx.operations.iter().collect::<Vec<_>>(),
            )
        }
    }
}

pub fn build_tx_json(
    env: &TransactionEnvelope,
    tx_result: Option<&TransactionResult>,
    tx_meta: Option<&TransactionMeta>,
    limits: &Limits,
    ledger_id: u32,
    close_time: u64,
    tx_hash_hex: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let (from_str, max_fee_stroops, operations) = envelope_sender_and_ops(env);
    let size_bytes = env.to_xdr(limits.clone()).map(|v| v.len()).unwrap_or(0);
    let (fee_charged, status) = match tx_result {
        Some(tr) => (
            Some(tr.fee_charged),
            Some(transaction_result_status_name(&tr.result)),
        ),
        None => (None, None),
    };

    let operations_json: Vec<serde_json::Value> = operations
        .iter()
        .enumerate()
        .map(|(i, op)| {
            let mut op_obj = json!({ "index": i, "name": op.body.name() });
            if let OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function, ..
            }) = &op.body
            {
                if let HostFunction::InvokeContract(args) = host_function {
                    op_obj["to"] = json!(args.contract_address.to_string());
                    op_obj["method"] = json!(
                        std::str::from_utf8(args.function_name.0.as_slice())
                            .unwrap_or("<non-utf8>")
                    );
                    let params: Vec<serde_json::Value> = args
                        .args
                        .iter()
                        .map(scval_to_typed_json_value)
                        .collect();
                    op_obj["parameters"] = json!(params);
                }
            }
            op_obj
        })
        .collect();

    let events_json = tx_meta
        .map(events_from_tx_meta_to_json)
        .unwrap_or_default();

    let mut root = json!({
        "ledger_id": ledger_id,
        "tx_hash": tx_hash_hex,
        "from": from_str,
        "timestamp": close_time,
        "max_fee_stroops": max_fee_stroops,
        "transaction_size_bytes": size_bytes,
    });
    if let Some(res) = soroban_resources_from_envelope(env) {
        root["soroban_resources_invoke"] = res;
    }
    if let Some(f) = fee_charged {
        root["fee_charged"] = json!(f);
    }
    if let Some(s) = status {
        root["status"] = json!(s);
    }
    root["operations"] = json!(operations_json);
    root["events"] = json!(events_json);
    Ok(root)
}
```
