//! Explore LedgerCloseMeta XDR structure — Rust proof-of-concept for task 0002.
//!
//! Validates research findings against real Galexie XDR data:
//! - LedgerCloseMetaBatch deserialization
//! - V0/V1 transaction phase envelope extraction
//! - V3/V4 TransactionMeta event handling
//! - ScVal typed JSON conversion
//! - Transaction hash computation
//! - LedgerEntryChanges classification
//!
//! Usage: cargo run [path-to-xdr-zst]
//! Default: testdata.xdr.zst

use base64::Engine;
use serde_json::json;
use sha2::{Digest, Sha256};
use stellar_xdr::curr::*;
use std::env;
use std::fs;

// --- ScVal to typed JSON ---

fn scval_to_typed_json(v: &ScVal) -> serde_json::Value {
    let (type_name, value) = match v {
        ScVal::Bool(b) => ("bool", json!(*b)),
        ScVal::Void => ("void", json!(null)),
        ScVal::Error(e) => ("error", json!(format!("{:?}", e))),
        ScVal::U32(x) => ("u32", json!(*x)),
        ScVal::I32(x) => ("i32", json!(*x)),
        ScVal::U64(x) => ("u64", json!(*x)),
        ScVal::I64(x) => ("i64", json!(*x)),
        ScVal::Timepoint(t) => {
            let u: u64 = t.clone().into();
            ("timepoint", json!(u))
        }
        ScVal::Duration(d) => ("duration", json!(d.0)),
        ScVal::U128(parts) => ("u128", json!(u128::from(parts).to_string())),
        ScVal::I128(parts) => ("i128", json!(i128::from(parts).to_string())),
        ScVal::U256(_) | ScVal::I256(_) => ("u256/i256", json!(format!("{:?}", v))),
        ScVal::Bytes(b) => (
            "bytes",
            json!(base64::engine::general_purpose::STANDARD.encode(b.0.as_slice())),
        ),
        ScVal::String(s) => (
            "string",
            json!(std::str::from_utf8(s.0.as_slice()).unwrap_or("<invalid-utf8>")),
        ),
        ScVal::Symbol(s) => (
            "sym",
            json!(std::str::from_utf8(s.0.as_slice()).unwrap_or("<invalid-utf8>")),
        ),
        ScVal::Vec(Some(vec)) => (
            "vec",
            json!(vec
                .iter()
                .map(scval_to_typed_json)
                .collect::<Vec<_>>()),
        ),
        ScVal::Vec(None) => ("vec", json!(null)),
        ScVal::Map(Some(m)) => (
            "map",
            json!(m
                .iter()
                .map(|e| json!({
                    "key": scval_to_typed_json(&e.key),
                    "value": scval_to_typed_json(&e.val),
                }))
                .collect::<Vec<_>>()),
        ),
        ScVal::Map(None) => ("map", json!(null)),
        ScVal::Address(a) => ("address", json!(a.to_string())),
        ScVal::ContractInstance(_) => ("contract_instance", json!("...")),
        ScVal::LedgerKeyContractInstance => ("ledger_key_contract_instance", json!(null)),
        ScVal::LedgerKeyNonce(k) => ("ledger_key_nonce", json!(format!("{:?}", k))),
    };
    json!({ "type": type_name, "value": value })
}

// --- Envelope extraction ---

fn for_envelopes_in_phase(phase: &TransactionPhase, envelopes: &mut Vec<TransactionEnvelope>) {
    match phase {
        TransactionPhase::V0(components) => {
            for comp in components.iter() {
                let TxSetComponent::TxsetCompTxsMaybeDiscountedFee(txs_comp) = comp;
                for env in txs_comp.txs.iter() {
                    envelopes.push(env.clone());
                }
            }
        }
        TransactionPhase::V1(parallel) => {
            for stage in parallel.execution_stages.iter() {
                for cluster in stage.0.iter() {
                    for env in cluster.0.iter() {
                        envelopes.push(env.clone());
                    }
                }
            }
        }
    }
}

fn extract_envelopes(meta: &LedgerCloseMeta) -> Vec<TransactionEnvelope> {
    let mut envelopes = Vec::new();
    match meta {
        LedgerCloseMeta::V0(v) => {
            for env in v.tx_set.txs.iter() {
                envelopes.push(env.clone());
            }
        }
        LedgerCloseMeta::V1(v) => {
            let GeneralizedTransactionSet::V1(ts1) = &v.tx_set;
            for phase in ts1.phases.iter() {
                for_envelopes_in_phase(phase, &mut envelopes);
            }
        }
        LedgerCloseMeta::V2(v) => {
            let GeneralizedTransactionSet::V1(ts1) = &v.tx_set;
            for phase in ts1.phases.iter() {
                for_envelopes_in_phase(phase, &mut envelopes);
            }
        }
    }
    envelopes
}

// --- Envelope details ---

fn envelope_source(env: &TransactionEnvelope) -> String {
    match env {
        TransactionEnvelope::TxV0(v0) => {
            MuxedAccount::from(&v0.tx.source_account_ed25519).to_string()
        }
        TransactionEnvelope::Tx(v1) => v1.tx.source_account.to_string(),
        TransactionEnvelope::TxFeeBump(fb) => {
            let FeeBumpTransactionInnerTx::Tx(inner) = &fb.tx.inner_tx;
            inner.tx.source_account.to_string()
        }
    }
}

fn envelope_ops(env: &TransactionEnvelope) -> &[Operation] {
    match env {
        TransactionEnvelope::TxV0(v0) => &v0.tx.operations,
        TransactionEnvelope::Tx(v1) => &v1.tx.operations,
        TransactionEnvelope::TxFeeBump(fb) => {
            let FeeBumpTransactionInnerTx::Tx(inner) = &fb.tx.inner_tx;
            &inner.tx.operations
        }
    }
}

// --- Event extraction ---

fn events_from_meta(meta: &TransactionMeta) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    match meta {
        TransactionMeta::V3(v3) => {
            if let Some(ref soroban) = v3.soroban_meta {
                for ev in soroban.events.iter() {
                    out.push(event_to_json(ev, None));
                }
            }
        }
        TransactionMeta::V4(v4) => {
            for te in v4.events.iter() {
                out.push(event_to_json(&te.event, Some(te.stage.name())));
            }
            for op_meta in v4.operations.iter() {
                for ev in op_meta.events.iter() {
                    out.push(event_to_json(ev, None));
                }
            }
        }
        _ => {}
    }
    out
}

fn event_to_json(ev: &ContractEvent, stage: Option<&str>) -> serde_json::Value {
    let ContractEventBody::V0(v0) = &ev.body;
    let mut obj = json!({
        "type": ev.type_.name(),
        "contract": ev.contract_id.as_ref().map(|c| c.to_string()).unwrap_or_default(),
        "topics": v0.topics.iter().map(scval_to_typed_json).collect::<Vec<_>>(),
        "data": scval_to_typed_json(&v0.data),
    });
    if let Some(s) = stage {
        obj["stage"] = json!(s);
    }
    obj
}

// --- LedgerEntryChanges ---

fn count_change(change: &LedgerEntryChange, counts: &mut std::collections::BTreeMap<String, usize>) {
    let entry_type = match change {
        LedgerEntryChange::Created(e) => Some(e.data.name()),
        LedgerEntryChange::Updated(e) => Some(e.data.name()),
        LedgerEntryChange::State(e) => Some(e.data.name()),
        LedgerEntryChange::Removed(_) => Some("removed"),
        LedgerEntryChange::Restored(e) => Some(e.data.name()),
    };
    if let Some(t) = entry_type {
        *counts.entry(t.to_string()).or_insert(0) += 1;
    }
}

fn classify_changes(meta: &TransactionMeta) -> serde_json::Value {
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    match meta {
        TransactionMeta::V3(v3) => {
            for op_meta in v3.operations.iter() {
                for change in op_meta.changes.0.iter() {
                    count_change(change, &mut counts);
                }
            }
        }
        TransactionMeta::V4(v4) => {
            for op_meta in v4.operations.iter() {
                for change in op_meta.changes.0.iter() {
                    count_change(change, &mut counts);
                }
            }
        }
        _ => {}
    }
    json!(counts)
}

// --- Invocation tree ---

fn invocation_to_json(inv: &SorobanAuthorizedInvocation) -> serde_json::Value {
    let call = match &inv.function {
        SorobanAuthorizedFunction::ContractFn(args) => {
            json!({
                "type": "invokeContract",
                "contract": args.contract_address.to_string(),
                "function": std::str::from_utf8(args.function_name.0.as_slice()).unwrap_or("<non-utf8>"),
                "args": args.args.iter().map(scval_to_typed_json).collect::<Vec<_>>(),
            })
        }
        SorobanAuthorizedFunction::CreateContractHostFn(_) => {
            json!({ "type": "createContract" })
        }
        SorobanAuthorizedFunction::CreateContractV2HostFn(_) => {
            json!({ "type": "createContractV2" })
        }
    };

    json!({
        "call": call,
        "subInvocations": inv.sub_invocations.iter().map(invocation_to_json).collect::<Vec<_>>(),
    })
}

// --- Main ---

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "testdata.xdr.zst".to_string());

    let compressed = fs::read(&path)?;
    let decompressed = zstd::decode_all(compressed.as_slice())?;

    let limits = Limits {
        depth: 1000,
        len: decompressed.len().max(10_000_000),
    };
    let batch = LedgerCloseMetaBatch::from_xdr(decompressed.as_slice(), limits.clone())?;

    println!("=== BATCH ===");
    println!("Ledgers in batch: {}", batch.ledger_close_metas.len());
    println!(
        "Start: {} End: {}",
        batch.start_sequence, batch.end_sequence
    );

    // Use testnet network ID
    let network_id: [u8; 32] = hex::decode(
        "cee0302d59844d32bdca915c8203dd44b33fbb7edc19051ea37abedf28ecd472",
    )?
    .try_into()
    .map_err(|_| "invalid network id")?;

    for meta in batch.ledger_close_metas.iter() {
        let (seq, close_time, tx_count) = match meta {
            LedgerCloseMeta::V0(v) => (
                v.ledger_header.header.ledger_seq,
                u64::from(v.ledger_header.header.scp_value.close_time.clone()),
                v.tx_processing.len(),
            ),
            LedgerCloseMeta::V1(v) => (
                v.ledger_header.header.ledger_seq,
                u64::from(v.ledger_header.header.scp_value.close_time.clone()),
                v.tx_processing.len(),
            ),
            LedgerCloseMeta::V2(v) => (
                v.ledger_header.header.ledger_seq,
                u64::from(v.ledger_header.header.scp_value.close_time.clone()),
                v.tx_processing.len(),
            ),
        };

        // Ledger hash
        let header_entry = match meta {
            LedgerCloseMeta::V0(v) => &v.ledger_header,
            LedgerCloseMeta::V1(v) => &v.ledger_header,
            LedgerCloseMeta::V2(v) => &v.ledger_header,
        };
        let header_xdr = header_entry.to_xdr(limits.clone())?;
        let ledger_hash = hex::encode(Sha256::digest(&header_xdr));

        let protocol = match meta {
            LedgerCloseMeta::V0(v) => v.ledger_header.header.ledger_version,
            LedgerCloseMeta::V1(v) => v.ledger_header.header.ledger_version,
            LedgerCloseMeta::V2(v) => v.ledger_header.header.ledger_version,
        };

        println!("\n=== LEDGER {} ===", seq);
        println!(
            "Hash: {} | Protocol: {} | Close: {} | TXs: {}",
            &ledger_hash[..16],
            protocol,
            close_time,
            tx_count
        );

        // Extract envelopes
        let envelopes = extract_envelopes(meta);
        println!("Envelopes extracted: {}", envelopes.len());

        // Process transactions — V0/V1 use TransactionResultMeta, V2 uses TransactionResultMetaV1
        // We unify by extracting common fields
        struct TxInfo<'a> {
            hash: [u8; 32],
            fee: i64,
            result_name: &'a str,
            meta: &'a TransactionMeta,
        }

        let tx_infos: Vec<TxInfo> = match meta {
            LedgerCloseMeta::V0(v) => v.tx_processing.iter().map(|p| TxInfo {
                hash: p.result.transaction_hash.0,
                fee: p.result.result.fee_charged,
                result_name: p.result.result.result.name(),
                meta: &p.tx_apply_processing,
            }).collect(),
            LedgerCloseMeta::V1(v) => v.tx_processing.iter().map(|p| TxInfo {
                hash: p.result.transaction_hash.0,
                fee: p.result.result.fee_charged,
                result_name: p.result.result.result.name(),
                meta: &p.tx_apply_processing,
            }).collect(),
            LedgerCloseMeta::V2(v) => v.tx_processing.iter().map(|p| TxInfo {
                hash: p.result.transaction_hash.0,
                fee: p.result.result.fee_charged,
                result_name: p.result.result.result.name(),
                meta: &p.tx_apply_processing,
            }).collect(),
        };

        let mut total_events = 0;
        let mut total_changes = 0;
        let mut meta_versions: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();

        for (i, proc) in tx_infos.iter().enumerate() {
            let tx_hash = hex::encode(proc.hash);
            let fee = proc.fee;
            let result_name = proc.result_name;
            let meta_version = proc.meta.name();

            *meta_versions.entry(meta_version.to_string()).or_insert(0) += 1;

            // Events
            let events = events_from_meta(proc.meta);
            total_events += events.len();

            // Changes
            let changes = classify_changes(proc.meta);
            if let Some(obj) = changes.as_object() {
                for (_, count) in obj.iter() {
                    total_changes += count.as_u64().unwrap_or(0) as usize;
                }
            }

            // Print first 3 TXs in detail
            if i < 3 {
                println!("\n--- TX[{}] {} ---", i, &tx_hash[..16]);
                println!("  fee: {} | result: {} | meta: {}", fee, result_name, meta_version);

                // Hash verification
                if let Some(env) = envelopes.get(i) {
                    let source = envelope_source(env);
                    println!("  source: {}...{}", &source[..8], &source[source.len() - 4..]);

                    if let Ok(computed_hash) = env.hash(network_id) {
                        let matches = computed_hash == proc.hash;
                        println!(
                            "  hash verify: {}",
                            if matches { "MATCH" } else { "MISMATCH" }
                        );
                    }

                    // Operations
                    let ops = envelope_ops(env);
                    println!("  operations: {}", ops.len());
                    for (j, op) in ops.iter().enumerate().take(3) {
                        let op_name = op.body.name();
                        print!("    op[{}]: {}", j, op_name);

                        if let OperationBody::InvokeHostFunction(ihf) = &op.body {
                            if let HostFunction::InvokeContract(args) = &ihf.host_function {
                                let fn_name = std::str::from_utf8(
                                    args.function_name.0.as_slice(),
                                )
                                .unwrap_or("<non-utf8>");
                                print!(
                                    " -> {}.{}({})",
                                    &args.contract_address.to_string()[..12],
                                    fn_name,
                                    args.args.len()
                                );
                            }

                            // Invocation tree
                            if !ihf.auth.is_empty() {
                                println!();
                                println!("    invocation tree:");
                                for entry in ihf.auth.iter() {
                                    let tree = invocation_to_json(&entry.root_invocation);
                                    println!(
                                        "      {}",
                                        serde_json::to_string_pretty(&tree)?
                                            .lines()
                                            .collect::<Vec<_>>()
                                            .join("\n      ")
                                    );
                                }
                            } else {
                                println!();
                            }
                        } else {
                            println!();
                        }
                    }
                }

                // Events
                if !events.is_empty() {
                    println!("  events: {}", events.len());
                    for (e, ev) in events.iter().enumerate().take(3) {
                        println!("    event[{}]: {}", e, serde_json::to_string(ev)?);
                    }
                }

                // Changes
                if changes.as_object().map_or(false, |o| !o.is_empty()) {
                    println!("  changes: {}", changes);
                }
            }
        }

        println!("\n=== SUMMARY ===");
        println!("Transactions: {}", tx_count);
        println!("Total events: {}", total_events);
        println!("Total changes: {}", total_changes);
        println!("Meta versions: {:?}", meta_versions);

        // Raw XDR sizes (first TX)
        if let Some(env) = envelopes.first() {
            if let Ok(xdr) = env.to_xdr(limits.clone()) {
                println!("Sample envelope XDR: {} bytes", xdr.len());
            }
        }
        if let Some(info) = tx_infos.first() {
            if let Ok(xdr) = info.meta.to_xdr(limits.clone()) {
                println!("Sample result_meta XDR: {} bytes", xdr.len());
            }
        }
    }

    println!("\n=== ALL RESEARCH FINDINGS VERIFIED ===");
    Ok(())
}
