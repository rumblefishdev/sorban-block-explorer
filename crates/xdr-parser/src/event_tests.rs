use super::*;

/// Real mainnet `executable_update` topics from contract CCABO2IQ… (first
/// upgrade, ledger 55363489): old `8b89f74f…`, new `55827b34…`. The fn must
/// return the NEW hash.
#[test]
fn extracts_new_wasm_hash_from_executable_update_topics() {
    let topics: Value = serde_json::from_str(
        r#"[{"type":"sym","value":"executable_update"},{"type":"vec","value":[{"type":"sym","value":"Wasm"},{"type":"bytes","value":"i4n3TxyvZkB6DzwJPtmvBXoW5c9VvuVtd56kVKbDCxw="}]},{"type":"vec","value":[{"type":"sym","value":"Wasm"},{"type":"bytes","value":"VYJ7NLoW/zBUeWZF4L0xpQ6HGUzBI9zdjg8i56LMnZg="}]}]"#,
    )
    .unwrap();
    let got = extract_executable_update_new_wasm_hash(&topics).expect("new wasm hash");
    let got_hex: String = got.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        got_hex,
        "55827b34ba16ff3054796645e0bd31a50e87194cc123dcdd8e0f22e7a2cc9d98"
    );
}

#[test]
fn ignores_non_executable_update_event() {
    // A `transfer` event must never yield a hash — guards the backfill from
    // mis-firing on the ~9.25B non-upgrade events.
    let topics: Value = serde_json::from_str(
        r#"[{"type":"sym","value":"transfer"},{"type":"address","value":"GAHWFCCYCDSSEYZRVV4446KBDP6X2JR2T56TG4OGANS3AS3NCILOCGU5"}]"#,
    )
    .unwrap();
    assert_eq!(extract_executable_update_new_wasm_hash(&topics), None);
}

#[test]
fn ignores_non_wasm_executable() {
    // Defensive: a StellarAsset (SAC) executable carries no wasm hash.
    let topics: Value = serde_json::from_str(
        r#"[{"type":"sym","value":"executable_update"},{"type":"vec","value":[{"type":"sym","value":"StellarAsset"}]},{"type":"vec","value":[{"type":"sym","value":"StellarAsset"}]}]"#,
    )
    .unwrap();
    assert_eq!(extract_executable_update_new_wasm_hash(&topics), None);
}

/// Round-trip through the REAL `scval_to_typed_json` encoder (not a
/// hand-written base64 literal) so the parser stays pinned to how the
/// indexer actually serializes `Bytes`/`Vec`/`Symbol` topics — a future
/// change to that encoding breaks this test in CI instead of silently
/// regressing upgrade detection to "unparseable".
#[test]
fn extracts_new_hash_via_real_scval_encoding() {
    use crate::scval::scval_to_typed_json;
    let sym = |s: &str| ScVal::Symbol(ScSymbol::try_from(s.as_bytes().to_vec()).unwrap());
    let exec = |b: u8| {
        ScVal::Vec(Some(
            ScVec::try_from(vec![
                sym("Wasm"),
                ScVal::Bytes(ScBytes::try_from(vec![b; 32]).unwrap()),
            ])
            .unwrap(),
        ))
    };
    let topics = serde_json::json!([
        scval_to_typed_json(&sym("executable_update")),
        scval_to_typed_json(&exec(0x11)),
        scval_to_typed_json(&exec(0x99)),
    ]);
    assert_eq!(
        extract_executable_update_new_wasm_hash(&topics),
        Some([0x99u8; 32])
    );
}

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
fn extract_events_v4_carries_transaction_event_stage() {
    // CAP-67 gives ONLY tx-level events a stage, and it is the protocol's
    // one statement of when they fired. `AfterTx` here is a synthetic
    // value chosen to prove the field is carried verbatim — real mainnet
    // refunds arrive as `AfterAllTxs`, pinned in
    // `tests/tx_event_stage_real_meta.rs`. The second event is numbered 1,
    // ahead of the operation event at 2 that it refunds —
    // which is exactly why `event_index` must not be read as a timeline.
    let charge = TransactionEvent {
        stage: TransactionEventStage::BeforeAllTxs,
        event: make_contract_event(0xAA, 10),
    };
    let refund = TransactionEvent {
        stage: TransactionEventStage::AfterTx,
        event: make_contract_event(0xAA, 3),
    };
    let op = OperationMetaV2 {
        ext: ExtensionPoint::V0,
        changes: LedgerEntryChanges::default(),
        events: vec![make_contract_event(0xB0, 200)].try_into().unwrap(),
    };
    let diag = DiagnosticEvent {
        in_successful_contract_call: true,
        event: make_contract_event(0xDD, 400),
    };
    let tx_meta = make_v4_meta(vec![charge, refund], vec![op], vec![diag]);

    let events = extract_events(&tx_meta, "abc", 1, 0);

    assert_eq!(events[0].stage, Some(TransactionEventStage::BeforeAllTxs));
    assert_eq!(events[1].stage, Some(TransactionEventStage::AfterTx));
    // Per-op and diagnostic events carry no stage — nothing to invent.
    assert_eq!(events[2].stage, None, "per-op event has no stage");
    assert_eq!(events[2].op_index, Some(0));
    assert_eq!(events[3].stage, None, "diagnostic event has no stage");
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
            data: ScVal::String(ScString::try_from("error details".as_bytes().to_vec()).unwrap()),
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

// ---- Task 0182: source-container tagging --------------------------------
//
// The parser must tag every event with the container it came from
// (`EventSource::TxLevel | PerOp | Diagnostic`) so downstream consumers
// can drop the diagnostic_events container *as a whole* — including
// the byte-identical Contract-typed mirrors Stellar core copies into
// it — without trusting the inner `event.type_`.

#[test]
fn v3_source_tagging() {
    let make_event = |type_| ContractEvent {
        ext: ExtensionPoint::V0,
        contract_id: None,
        type_,
        body: ContractEventBody::V0(ContractEventV0 {
            topics: VecM::default(),
            data: ScVal::Void,
        }),
    };

    let soroban_meta = SorobanTransactionMeta {
        ext: SorobanTransactionMetaExt::V0,
        events: vec![make_event(ContractEventType::Contract)]
            .try_into()
            .unwrap(),
        return_value: ScVal::Void,
        diagnostic_events: vec![DiagnosticEvent {
            in_successful_contract_call: false,
            event: make_event(ContractEventType::Diagnostic),
        }]
        .try_into()
        .unwrap(),
    };

    let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
        ext: ExtensionPoint::V0,
        tx_changes_before: LedgerEntryChanges::default(),
        operations: VecM::default(),
        tx_changes_after: LedgerEntryChanges::default(),
        soroban_meta: Some(soroban_meta),
    });

    let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].source, EventSource::TxLevel);
    assert_eq!(events[1].source, EventSource::Diagnostic);
}

#[test]
fn v4_source_tagging_three_locations() {
    // One event in each of v4.events / v4.operations[0].events /
    // v4.diagnostic_events. Sources must come out TxLevel / PerOp /
    // Diagnostic respectively.
    let tx_event = TransactionEvent {
        stage: TransactionEventStage::default(),
        event: make_contract_event(0xAA, 1),
    };
    let op = OperationMetaV2 {
        ext: ExtensionPoint::V0,
        changes: LedgerEntryChanges::default(),
        events: vec![make_contract_event(0xBB, 2)].try_into().unwrap(),
    };
    let diag = DiagnosticEvent {
        in_successful_contract_call: false,
        event: make_contract_event(0xCC, 3),
    };
    let tx_meta = make_v4_meta(vec![tx_event], vec![op], vec![diag]);

    let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].source, EventSource::TxLevel);
    assert_eq!(events[1].source, EventSource::PerOp);
    assert_eq!(events[2].source, EventSource::Diagnostic);
    // Only the per-op container carries the operation attribution (D7).
    assert_eq!(events[0].op_index, None);
    assert_eq!(events[1].op_index, Some(0));
    assert_eq!(events[2].op_index, None);
}

#[test]
fn v4_diag_contract_typed_mirror_tagged_diagnostic() {
    // Reproduces the task-0182 bug surface: Stellar core mirrors a
    // per-op Contract event into v4.diagnostic_events with the *same*
    // inner type_ = Contract. Filtering by `event_type` cannot tell
    // the original from the mirror; only `source` can.
    let original = make_contract_event(0xAA, 42);
    let mirror = make_contract_event(0xAA, 42); // byte-identical

    let op = OperationMetaV2 {
        ext: ExtensionPoint::V0,
        changes: LedgerEntryChanges::default(),
        events: vec![original].try_into().unwrap(),
    };
    let diag = DiagnosticEvent {
        in_successful_contract_call: true,
        event: mirror,
    };
    let tx_meta = make_v4_meta(Vec::new(), vec![op], vec![diag]);

    let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
    assert_eq!(events.len(), 2);

    // Both are Contract by inner type.
    assert_eq!(events[0].event_type, DomainEventType::Contract);
    assert_eq!(events[1].event_type, DomainEventType::Contract);

    // But sources differ — that's the only signal that lets staging
    // and read-time API drop the mirror.
    assert_eq!(events[0].source, EventSource::PerOp);
    assert_eq!(events[1].source, EventSource::Diagnostic);
}

#[test]
fn v4_multi_op_per_op_events_all_tagged_per_op() {
    let op0 = OperationMetaV2 {
        ext: ExtensionPoint::V0,
        changes: LedgerEntryChanges::default(),
        events: vec![make_contract_event(0xB0, 1), make_contract_event(0xB1, 2)]
            .try_into()
            .unwrap(),
    };
    let op1 = OperationMetaV2 {
        ext: ExtensionPoint::V0,
        changes: LedgerEntryChanges::default(),
        events: vec![make_contract_event(0xC0, 3)].try_into().unwrap(),
    };
    let tx_meta = make_v4_meta(Vec::new(), vec![op0, op1], Vec::new());

    let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
    assert_eq!(events.len(), 3);
    assert!(
        events.iter().all(|e| e.source == EventSource::PerOp),
        "every per-op event across operations must be tagged PerOp"
    );
    // Attribution follows the envelope position of the emitting op (D7).
    assert_eq!(
        events.iter().map(|e| e.op_index).collect::<Vec<_>>(),
        vec![Some(0), Some(0), Some(1)]
    );
    // The position inside the operation resets per operation — this is the
    // `event` component of the official `(ledger, tx, op, event)` identity
    // (task 0540), NOT our flat `event_index`.
    assert_eq!(
        events.iter().map(|e| e.event_pos_in_op).collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(0)]
    );
    assert_eq!(
        events.iter().map(|e| e.event_index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn v4_event_pos_in_op_is_none_outside_the_per_op_container() {
    let tx_event = TransactionEvent {
        stage: TransactionEventStage::BeforeAllTxs,
        event: make_contract_event(0xAA, 7),
    };
    let op = OperationMetaV2 {
        ext: ExtensionPoint::V0,
        changes: LedgerEntryChanges::default(),
        events: vec![make_contract_event(0xB0, 1)].try_into().unwrap(),
    };
    let diag = DiagnosticEvent {
        in_successful_contract_call: true,
        event: make_contract_event(0xDD, 9),
    };
    let tx_meta = make_v4_meta(vec![tx_event], vec![op], vec![diag]);

    let events = extract_events(&tx_meta, "abcd1234", 100, 1700000000);
    assert_eq!(
        events
            .iter()
            .map(|e| (e.op_index, e.event_pos_in_op))
            .collect::<Vec<_>>(),
        vec![(None, None), (Some(0), Some(0)), (None, None)],
        "only the per-op container carries the official identity"
    );
}
