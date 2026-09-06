//! CAP-67 event extraction from Soroban transaction metadata.
//!
//! Extracts contract, system, and diagnostic events from `SorobanTransactionMeta`.
//! Each event is decoded into an `ExtractedEvent` with ScVal-decoded topics and data.

use base64::Engine;
use serde_json::{Value, json};
use stellar_xdr::*;

use crate::scval::scval_to_typed_json;
use crate::types::{EventSource, ExtractedEvent};
use domain::ContractEventType as DomainEventType;

/// Extract all events from a transaction's metadata.
///
/// Returns one `ExtractedEvent` per event in `SorobanTransactionMeta.events`.
/// Returns an empty vec for non-Soroban transactions (no V3/V4 meta).
pub fn extract_events(
    tx_meta: &TransactionMeta,
    transaction_hash: &str,
    ledger_sequence: u32,
    created_at: i64,
) -> Vec<ExtractedEvent> {
    match tx_meta {
        TransactionMeta::V3(v3) => {
            let Some(ref meta) = v3.soroban_meta else {
                return Vec::new();
            };
            let mut extracted: Vec<ExtractedEvent> = meta
                .events
                .iter()
                .enumerate()
                .map(|(i, event)| {
                    extract_single_event(
                        event,
                        transaction_hash,
                        ledger_sequence,
                        created_at,
                        i,
                        EventSource::TxLevel,
                    )
                })
                .collect();
            // Include diagnostic_events (separate field in SorobanTransactionMeta)
            let base = extracted.len();
            for (j, diag) in meta.diagnostic_events.iter().enumerate() {
                extracted.push(extract_single_event(
                    &diag.event,
                    transaction_hash,
                    ledger_sequence,
                    created_at,
                    base + j,
                    EventSource::Diagnostic,
                ));
            }
            extracted
        }
        TransactionMeta::V4(v4) => {
            // CAP-67 (Protocol 23+) reorganises events into three locations
            // — tx-level (fee charge and refund, carrying a `stage`),
            // per-operation (Soroban contract events emitted during
            // InvokeHostFunction execution + classic-op SAC events under
            // Protocol 23 unification), and diagnostic. `event_index` is
            // numbered sequentially across all three sources so the V3
            // contract (monotonic per-tx index) is preserved. Each event
            // is tagged with its `EventSource` so consumers can drop the
            // diagnostic container without trusting the inner `type_`
            // (the diagnostic container holds byte-identical Contract-typed
            // copies of per-op consensus events when diagnostic mode is
            // enabled — task 0182).
            let mut extracted: Vec<ExtractedEvent> = v4
                .events
                .iter()
                .enumerate()
                .map(|(i, tx_event)| {
                    // CAP-67 gives tx-level events a `stage`, and it is the
                    // only statement of WHEN they happened. Measured on
                    // mainnet, not inferred (`tests/tx_event_stage_real_meta`):
                    // the fee charge is `BeforeAllTxs`, the refund
                    // `AfterAllTxs` — settled after every transaction in the
                    // ledger, yet numbered ahead of the operation it refunds.
                    // Without the stage, `event_index` reads as a timeline it
                    // is not.
                    let mut ev = extract_single_event(
                        &tx_event.event,
                        transaction_hash,
                        ledger_sequence,
                        created_at,
                        i,
                        EventSource::TxLevel,
                    );
                    ev.stage = Some(tx_event.stage);
                    ev
                })
                .collect();

            let mut next_idx = extracted.len();
            for (op_i, op_meta) in v4.operations.iter().enumerate() {
                for (pos, event) in op_meta.events.iter().enumerate() {
                    let mut ev = extract_single_event(
                        event,
                        transaction_hash,
                        ledger_sequence,
                        created_at,
                        next_idx,
                        EventSource::PerOp,
                    );
                    // Keep the envelope position — it is the only place the
                    // meta states which operation emitted the event (D7) —
                    // and the position inside that operation, which with it
                    // forms the official event identity (task 0540).
                    ev.op_index = u32::try_from(op_i).ok();
                    ev.event_pos_in_op = u32::try_from(pos).ok();
                    extracted.push(ev);
                    next_idx += 1;
                }
            }

            for diag in v4.diagnostic_events.iter() {
                extracted.push(extract_single_event(
                    &diag.event,
                    transaction_hash,
                    ledger_sequence,
                    created_at,
                    next_idx,
                    EventSource::Diagnostic,
                ));
                next_idx += 1;
            }

            extracted
        }
        _ => Vec::new(),
    }
}

/// Extract a single `ContractEvent` into an `ExtractedEvent`.
fn extract_single_event(
    event: &ContractEvent,
    transaction_hash: &str,
    ledger_sequence: u32,
    created_at: i64,
    index: usize,
    source: EventSource,
) -> ExtractedEvent {
    // ADR 0031: emit the typed enum directly; persist binds it as SMALLINT.
    let event_type = match event.type_ {
        ContractEventType::System => DomainEventType::System,
        ContractEventType::Contract => DomainEventType::Contract,
        ContractEventType::Diagnostic => DomainEventType::Diagnostic,
    };

    let contract_id = event
        .contract_id
        .as_ref()
        .map(|id| ScAddress::Contract(id.clone()).to_string());

    let (topics, data) = match &event.body {
        ContractEventBody::V0(v0) => {
            let topics: Vec<Value> = v0.topics.iter().map(scval_to_typed_json).collect();
            let data = scval_to_typed_json(&v0.data);
            (json!(topics), data)
        }
    };

    ExtractedEvent {
        transaction_hash: transaction_hash.to_string(),
        event_type,
        source,
        contract_id,
        topics,
        data,
        event_index: u32::try_from(index).expect("event index does not fit into u32"),
        op_index: None,
        event_pos_in_op: None,
        // Only `v4.events` carries one; the tx-level arm sets it.
        stage: None,
        ledger_sequence,
        created_at,
    }
}

/// Extract the NEW executable WASM hash from a decoded `executable_update`
/// system-event `topics` array (the typed-JSON stored in
/// `soroban_events.topics_xdr`).
///
/// Per CAP-0046-05 a WASM upgrade emits a system event whose topics are
/// `[Symbol("executable_update"), <old executable>, <new executable>]`, where
/// each executable is `vec[Symbol("Wasm"), Bytes(hash)]`. Returns the new
/// 32-byte hash, or `None` if the shape doesn't match — wrong topic name,
/// a non-`Wasm` executable (e.g. a SAC `StellarAsset`, which never upgrades),
/// or a hash that isn't exactly 32 bytes.
pub fn extract_executable_update_new_wasm_hash(topics: &Value) -> Option<[u8; 32]> {
    let arr = topics.as_array()?;
    // topic[0] is the event-name symbol.
    if arr.first()?.get("value")?.as_str()? != "executable_update" {
        return None;
    }
    // topic[2] is the NEW executable: vec[Symbol("Wasm"), Bytes(hash)].
    let new_exec = arr.get(2)?.get("value")?.as_array()?;
    if new_exec.first()?.get("value")?.as_str()? != "Wasm" {
        return None; // e.g. a StellarAsset executable — SACs never upgrade.
    }
    let b64 = new_exec.get(1)?.get("value")?.as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    bytes.try_into().ok()
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
