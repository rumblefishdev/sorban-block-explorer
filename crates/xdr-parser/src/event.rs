//! CAP-67 event extraction from Soroban transaction metadata.
//!
//! Extracts contract, system, and diagnostic events from `SorobanTransactionMeta`.
//! Each event is decoded into an `ExtractedEvent` with ScVal-decoded topics and data.

use serde_json::{Value, json};
use stellar_xdr::curr::*;

use crate::scval::scval_to_typed_json;
use crate::types::ExtractedEvent;
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
                    extract_single_event(event, transaction_hash, ledger_sequence, created_at, i)
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
                ));
            }
            extracted
        }
        TransactionMeta::V4(v4) => {
            // CAP-67 (Protocol 23+) reorganises events into three locations
            // — tx-level (fee BeforeAllTxs / AfterTx refund / AfterAllTxs),
            // per-operation (Soroban contract events emitted during
            // InvokeHostFunction execution + classic-op SAC events under
            // Protocol 23 unification), and diagnostic. `event_index` is
            // numbered sequentially across all three sources so the V3
            // contract (monotonic per-tx index) is preserved.
            let mut extracted: Vec<ExtractedEvent> = v4
                .events
                .iter()
                .enumerate()
                .map(|(i, tx_event)| {
                    extract_single_event(
                        &tx_event.event,
                        transaction_hash,
                        ledger_sequence,
                        created_at,
                        i,
                    )
                })
                .collect();

            let mut next_idx = extracted.len();
            for op_meta in v4.operations.iter() {
                for event in op_meta.events.iter() {
                    extracted.push(extract_single_event(
                        event,
                        transaction_hash,
                        ledger_sequence,
                        created_at,
                        next_idx,
                    ));
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
        contract_id,
        topics,
        data,
        event_index: u32::try_from(index).expect("event index does not fit into u32"),
        ledger_sequence,
        created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_contract_event() {
        let contract_id = Hash([0xAA; 32]);
        let topic = ScVal::Symbol(ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap());
        let data = ScVal::U64(42);

        let event = ContractEvent {
            ext: ExtensionPoint::V0,
            contract_id: Some(ContractId(contract_id)),
            type_: ContractEventType::Contract,
            body: ContractEventBody::V0(ContractEventV0 {
                topics: vec![topic].try_into().unwrap(),
                data,
            }),
        };

        let soroban_meta = SorobanTransactionMeta {
            ext: SorobanTransactionMetaExt::V0,
            events: vec![event].try_into().unwrap(),
            return_value: ScVal::Void,
            diagnostic_events: VecM::default(),
        };

        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: Some(soroban_meta),
        });

        let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
        assert_eq!(events.len(), 1);

        let e = &events[0];
        assert_eq!(e.event_type, DomainEventType::Contract);
        assert_eq!(e.transaction_hash, "abcd1234");
        assert!(e.contract_id.is_some());
        assert!(e.contract_id.as_ref().unwrap().starts_with('C'));
        assert_eq!(e.event_index, 0);
        assert_eq!(e.ledger_sequence, 100);
        assert_eq!(e.created_at, 1700000000);

        // Topics
        let topics = e.topics.as_array().unwrap();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0]["type"], "sym");
        assert_eq!(topics[0]["value"], "transfer");

        // Data
        assert_eq!(e.data["type"], "u64");
        assert_eq!(e.data["value"], 42);
    }

    #[test]
    fn extract_system_event_no_contract() {
        let event = ContractEvent {
            ext: ExtensionPoint::V0,
            contract_id: None,
            type_: ContractEventType::System,
            body: ContractEventBody::V0(ContractEventV0 {
                topics: VecM::default(),
                data: ScVal::Void,
            }),
        };

        let soroban_meta = SorobanTransactionMeta {
            ext: SorobanTransactionMetaExt::V0,
            events: vec![event].try_into().unwrap(),
            return_value: ScVal::Void,
            diagnostic_events: VecM::default(),
        };

        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: Some(soroban_meta),
        });

        let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, DomainEventType::System);
        assert!(events[0].contract_id.is_none());
        assert_eq!(events[0].topics.as_array().unwrap().len(), 0);
    }

    #[test]
    fn no_events_for_non_soroban_meta() {
        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
        assert!(events.is_empty());
    }

    #[test]
    fn multiple_events_preserve_order() {
        let make_event = |val: u32| ContractEvent {
            ext: ExtensionPoint::V0,
            contract_id: None,
            type_: ContractEventType::Contract,
            body: ContractEventBody::V0(ContractEventV0 {
                topics: VecM::default(),
                data: ScVal::U32(val),
            }),
        };

        let soroban_meta = SorobanTransactionMeta {
            ext: SorobanTransactionMetaExt::V0,
            events: vec![make_event(1), make_event(2), make_event(3)]
                .try_into()
                .unwrap(),
            return_value: ScVal::Void,
            diagnostic_events: VecM::default(),
        };

        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: Some(soroban_meta),
        });

        let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_index, 0);
        assert_eq!(events[0].data["value"], 1);
        assert_eq!(events[1].event_index, 1);
        assert_eq!(events[1].data["value"], 2);
        assert_eq!(events[2].event_index, 2);
        assert_eq!(events[2].data["value"], 3);
    }

    #[test]
    fn multiple_topics_decoded() {
        let topics = vec![
            ScVal::Symbol(ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap()),
            ScVal::Address(ScAddress::Contract(ContractId(Hash([0xBB; 32])))),
            ScVal::U64(100),
        ];

        let event = ContractEvent {
            ext: ExtensionPoint::V0,
            contract_id: Some(ContractId(Hash([0xAA; 32]))),
            type_: ContractEventType::Contract,
            body: ContractEventBody::V0(ContractEventV0 {
                topics: topics.try_into().unwrap(),
                data: ScVal::Void,
            }),
        };

        let soroban_meta = SorobanTransactionMeta {
            ext: SorobanTransactionMetaExt::V0,
            events: vec![event].try_into().unwrap(),
            return_value: ScVal::Void,
            diagnostic_events: VecM::default(),
        };

        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: Some(soroban_meta),
        });

        let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
        let topics = events[0].topics.as_array().unwrap();
        assert_eq!(topics.len(), 3);
        assert_eq!(topics[0]["type"], "sym");
        assert_eq!(topics[1]["type"], "address");
        assert_eq!(topics[2]["type"], "u64");
    }

    #[test]
    fn extract_events_from_v4_meta() {
        let event = ContractEvent {
            ext: ExtensionPoint::V0,
            contract_id: Some(ContractId(Hash([0xAA; 32]))),
            type_: ContractEventType::Contract,
            body: ContractEventBody::V0(ContractEventV0 {
                topics: VecM::default(),
                data: ScVal::U32(77),
            }),
        };

        let tx_event = TransactionEvent {
            stage: TransactionEventStage::default(),
            event,
        };

        let tx_meta = TransactionMeta::V4(TransactionMetaV4 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
            events: vec![tx_event].try_into().unwrap(),
            diagnostic_events: VecM::default(),
        });

        let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, DomainEventType::Contract);
        assert!(events[0].contract_id.is_some());
        assert_eq!(events[0].data["value"], 77);
    }

    // ---- CAP-67 / Protocol 23+ per-operation event coverage ----------------
    //
    // V4 events live in three locations (`v4.events`,
    // `v4.operations[i].events`, `v4.diagnostic_events`). The tests below
    // pin each pattern individually plus a mixed-sources case that proves
    // the iteration order (tx-level → per-op → diagnostic) and sequential
    // `event_index` numbering.

    fn make_contract_event(contract_byte: u8, data: u32) -> ContractEvent {
        ContractEvent {
            ext: ExtensionPoint::V0,
            contract_id: Some(ContractId(Hash([contract_byte; 32]))),
            type_: ContractEventType::Contract,
            body: ContractEventBody::V0(ContractEventV0 {
                topics: VecM::default(),
                data: ScVal::U32(data),
            }),
        }
    }

    fn make_v4_meta(
        tx_events: Vec<TransactionEvent>,
        operations: Vec<OperationMetaV2>,
        diagnostic_events: Vec<DiagnosticEvent>,
    ) -> TransactionMeta {
        TransactionMeta::V4(TransactionMetaV4 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: operations.try_into().unwrap(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
            events: tx_events.try_into().unwrap(),
            diagnostic_events: diagnostic_events.try_into().unwrap(),
        })
    }

    #[test]
    fn extract_events_v4_per_op_single() {
        // One operation carrying one contract event, no tx-level / diag.
        // Pre-fix this returned an empty vec; post-fix it returns the event.
        let op = OperationMetaV2 {
            ext: ExtensionPoint::V0,
            changes: LedgerEntryChanges::default(),
            events: vec![make_contract_event(0xCC, 11)].try_into().unwrap(),
        };
        let tx_meta = make_v4_meta(Vec::new(), vec![op], Vec::new());

        let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
        assert_eq!(events.len(), 1, "per-op event must be extracted");
        assert_eq!(events[0].event_type, DomainEventType::Contract);
        assert_eq!(events[0].event_index, 0);
        assert_eq!(events[0].data["value"], 11);
        assert!(events[0].contract_id.is_some());
    }

    #[test]
    fn extract_events_v4_mixed_sources_preserve_order_and_indexing() {
        // tx-level (1) + per-op across two operations (2 + 1) + diagnostic (1).
        // Expected order: tx-level → op0 → op1 → diagnostic, with sequential
        // `event_index` 0..=4 and contract bytes `0xAA, 0xB0, 0xB1, 0xC0, 0xDD`.
        let tx_event = TransactionEvent {
            stage: TransactionEventStage::default(),
            event: make_contract_event(0xAA, 100),
        };
        let op0 = OperationMetaV2 {
            ext: ExtensionPoint::V0,
            changes: LedgerEntryChanges::default(),
            events: vec![
                make_contract_event(0xB0, 200),
                make_contract_event(0xB1, 201),
            ]
            .try_into()
            .unwrap(),
        };
        let op1 = OperationMetaV2 {
            ext: ExtensionPoint::V0,
            changes: LedgerEntryChanges::default(),
            events: vec![make_contract_event(0xC0, 300)].try_into().unwrap(),
        };
        let diag = DiagnosticEvent {
            in_successful_contract_call: false,
            event: make_contract_event(0xDD, 400),
        };
        let tx_meta = make_v4_meta(vec![tx_event], vec![op0, op1], vec![diag]);

        let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
        assert_eq!(events.len(), 5, "1 tx-level + 3 per-op + 1 diagnostic");

        // Sequential event_index across the three sources.
        for (i, e) in events.iter().enumerate() {
            assert_eq!(
                e.event_index, i as u32,
                "event_index must be sequential across all sources"
            );
        }

        // Iteration order proven by the data values we planted per source.
        assert_eq!(events[0].data["value"], 100, "tx-level event first");
        assert_eq!(events[1].data["value"], 200, "op0 event[0] second");
        assert_eq!(events[2].data["value"], 201, "op0 event[1] third");
        assert_eq!(events[3].data["value"], 300, "op1 event[0] fourth");
        assert_eq!(events[4].data["value"], 400, "diagnostic event last");
    }

    #[test]
    fn extract_events_v4_empty_per_op_produces_no_spurious_rows() {
        // Two operations, both with empty events vec, plus one tx-level
        // event. Result must contain only the tx-level event — empty
        // OperationMetaV2.events must not advance the index or push
        // empty rows.
        let tx_event = TransactionEvent {
            stage: TransactionEventStage::default(),
            event: make_contract_event(0xAA, 7),
        };
        let empty_op = || OperationMetaV2 {
            ext: ExtensionPoint::V0,
            changes: LedgerEntryChanges::default(),
            events: VecM::default(),
        };
        let tx_meta = make_v4_meta(vec![tx_event], vec![empty_op(), empty_op()], Vec::new());

        let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_index, 0);
        assert_eq!(events[0].data["value"], 7);
    }

    #[test]
    fn events_extracted_regardless_of_tx_success() {
        // Events from a failed transaction should still be extracted.
        // Diagnostic events in particular are emitted even on failure.
        let event = ContractEvent {
            ext: ExtensionPoint::V0,
            contract_id: None,
            type_: ContractEventType::Diagnostic,
            body: ContractEventBody::V0(ContractEventV0 {
                topics: VecM::default(),
                data: ScVal::String(
                    ScString::try_from("error details".as_bytes().to_vec()).unwrap(),
                ),
            }),
        };

        let soroban_meta = SorobanTransactionMeta {
            ext: SorobanTransactionMetaExt::V0,
            events: vec![event].try_into().unwrap(),
            return_value: ScVal::Void,
            diagnostic_events: VecM::default(),
        };

        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: Some(soroban_meta),
        });

        let events = extract_events(&tx_meta, "failed_tx_hash", 100, 1700000000);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, DomainEventType::Diagnostic);
        assert_eq!(events[0].data["value"], "error details");
    }
}
