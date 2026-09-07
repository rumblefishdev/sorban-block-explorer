use super::*;
use domain::{ContractEventType, OperationType};
use serde_json::json;
use xdr_parser::EventAsset;

const TX: &str = "0a120260ab2a4d3e7f9c1b5d6e8f0a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1";
const G_SENDER: &str = "GARNRDKOUGVQ6FMJLL5RPDU6NG36LZHTEOIAWZPNOKFNKNKCOJEC3WMZ";
const G_RECEIVER: &str = "GD5SL5RIC5STHGDDOJGSIIZHZZQPA4HIYFEPIQ3FF7M3H2F5VQ7K2FTL";
/// `G_RECEIVER` + id 3539365402, built and round-tripped in the research note.
const M_RECEIVER: &str = "MD5SL5RIC5STHGDDOJGSIIZHZZQPA4HIYFEPIQ3FF7M3H2F5VQ7K2AAAAAANF5TODJEYA";
const POOL_L: &str = "LA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ";
const XLM_SAC: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";

fn tx(memo: Option<(&str, &str)>, source_muxed_id: Option<u64>) -> ExtractedTransaction {
    ExtractedTransaction {
        hash: TX.into(),
        inner_tx_hash: None,
        ledger_sequence: 64_259_660,
        source_account: G_SENDER.into(),
        source_muxed_id,
        fee_source: None,
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".into(),
        envelope_xdr: String::new(),
        result_xdr: String::new(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: memo.map(|m| m.0.to_string()),
        memo: memo.map(|m| m.1.to_string()),
        created_at: 0,
        parse_error: false,
        ledger_deltas: Vec::new(),
    }
}

fn payment_op(
    index: u32,
    destination: &str,
    destination_muxed_id: Option<u64>,
    source: Option<(&str, Option<u64>)>,
) -> ExtractedOperation {
    ExtractedOperation {
        transaction_hash: TX.into(),
        operation_index: index,
        op_type: OperationType::Payment,
        source_account: source.map(|s| s.0.to_string()),
        details: json!({ "destination": destination, "asset": "native", "amount": 10000 }),
        asset_appearances: Vec::new(),
        counterparties: Vec::new(),
        source_muxed_id: source.and_then(|s| s.1),
        destination_muxed_id,
    }
}

fn transfer(
    op_index: u32,
    pos: u32,
    event_index: u32,
    from: Option<&str>,
    to: Option<&str>,
    kind: TokenEventKind,
) -> ExtractedAssetTransfer {
    ExtractedAssetTransfer {
        transaction_hash: TX.into(),
        event_index,
        op_index,
        event_pos_in_op: pos,
        kind,
        from: from.map(str::to_string),
        to: to.map(str::to_string),
        asset: EventAsset::Native,
        emitter: XLM_SAC.into(),
        amount: Some(10_000),
    }
}

fn event(source: EventSource, event_index: u32, op: Option<(u32, u32)>) -> ExtractedEvent {
    ExtractedEvent {
        transaction_hash: TX.into(),
        event_type: ContractEventType::Contract,
        source,
        contract_id: Some(XLM_SAC.into()),
        topics: json!([]),
        data: json!({ "type": "void" }),
        event_index,
        op_index: op.map(|o| o.0),
        event_pos_in_op: op.map(|o| o.1),
        stage: None,
        ledger_sequence: 64_259_660,
        created_at: 0,
    }
}

#[test]
fn plain_payment_row_carries_ids_kinds_and_the_official_identity() {
    let txs = [tx(None, None)];
    let ops = [(TX.to_string(), vec![payment_op(1, G_RECEIVER, None, None)])];
    let t = [transfer(
        0,
        0,
        2,
        Some(G_SENDER),
        Some(G_RECEIVER),
        TokenEventKind::Transfer,
    )];

    let out = build_value_flow_rows(64_259_660, &txs, &ops, &[], &t).unwrap();
    assert_eq!(
        out.transfers,
        vec![AssetTransferRow {
            ledger_sequence: 64_259_660,
            application_order: 1,
            op_index: 0,
            event_pos_in_op: 0,
            event_index: 2,
            asset_id: ids::NATIVE_ASSET_ID,
            amount: Some(10_000),
            from_id: Some(ids::account_id(G_SENDER)),
            from_kind: "G".into(),
            from_muxed_id: None,
            to_id: Some(ids::account_id(G_RECEIVER)),
            to_kind: "G".into(),
            to_muxed_id: None,
            verb: "transfer".into(),
        }]
    );
    assert!(out.memos.is_empty());
}

#[test]
fn muxed_destination_comes_from_the_envelope_and_keeps_the_g_surrogate() {
    // The event says `to = G…` (CAP-67); the envelope's Payment says `M…`.
    let txs = [tx(None, None)];
    let ops = [(
        TX.to_string(),
        vec![payment_op(1, G_RECEIVER, Some(3_539_365_402), None)],
    )];
    let t = [transfer(
        0,
        0,
        2,
        Some(G_SENDER),
        Some(G_RECEIVER),
        TokenEventKind::Transfer,
    )];

    let row = &build_value_flow_rows(64_259_660, &txs, &ops, &[], &t)
        .unwrap()
        .transfers[0];
    assert_eq!(
        row.to_id,
        Some(ids::account_id(G_RECEIVER)),
        "hash of the G, never of the M"
    );
    assert_eq!(row.to_muxed_id, Some(3_539_365_402));
}

#[test]
fn muxed_id_is_not_borrowed_from_an_op_whose_destination_is_someone_else() {
    // Path payment: the op destination is G_RECEIVER (muxed), but this transfer
    // is an intermediate hop to a pool. The pool must not inherit the id.
    let txs = [tx(None, None)];
    let ops = [(
        TX.to_string(),
        vec![payment_op(1, G_RECEIVER, Some(42), None)],
    )];
    let t = [transfer(
        0,
        1,
        3,
        Some(G_SENDER),
        Some(POOL_L),
        TokenEventKind::Transfer,
    )];

    let row = &build_value_flow_rows(64_259_660, &txs, &ops, &[], &t)
        .unwrap()
        .transfers[0];
    assert_eq!(row.to_kind, "L");
    assert_eq!(row.to_muxed_id, None);
}

#[test]
fn muxed_source_comes_from_the_op_override_or_the_tx_source() {
    let txs = [tx(None, Some(7))];
    let ops = [(
        TX.to_string(),
        vec![
            payment_op(1, G_RECEIVER, None, None),
            payment_op(2, G_RECEIVER, None, Some((G_RECEIVER, Some(9)))),
        ],
    )];
    let t = [
        // op 0 inherits the tx source (muxed id 7)
        transfer(
            0,
            0,
            0,
            Some(G_SENDER),
            Some(G_RECEIVER),
            TokenEventKind::Transfer,
        ),
        // op 1 overrides the source with G_RECEIVER (muxed id 9); a transfer
        // FROM the tx source inside it must not get 7 or 9.
        transfer(
            1,
            0,
            1,
            Some(G_RECEIVER),
            Some(G_SENDER),
            TokenEventKind::Transfer,
        ),
        transfer(
            1,
            1,
            2,
            Some(G_SENDER),
            Some(G_RECEIVER),
            TokenEventKind::Transfer,
        ),
    ];

    let rows = build_value_flow_rows(64_259_660, &txs, &ops, &[], &t)
        .unwrap()
        .transfers;
    assert_eq!(rows[0].from_muxed_id, Some(7));
    assert_eq!(rows[1].from_muxed_id, Some(9));
    assert_eq!(rows[2].from_muxed_id, None);
}

#[test]
fn an_m_address_in_the_topic_itself_is_split() {
    let txs = [tx(None, None)];
    let t = [transfer(
        0,
        0,
        0,
        Some(G_SENDER),
        Some(M_RECEIVER),
        TokenEventKind::Transfer,
    )];
    let row = &build_value_flow_rows(64_259_660, &txs, &[], &[], &t)
        .unwrap()
        .transfers[0];
    assert_eq!(row.to_id, Some(ids::account_id(G_RECEIVER)));
    assert_eq!(row.to_kind, "G");
    assert_eq!(row.to_muxed_id, Some(3_539_365_402));
}

#[test]
fn mint_and_burn_leave_the_missing_end_null() {
    let txs = [tx(None, None)];
    let t = [
        transfer(0, 0, 0, None, Some(G_RECEIVER), TokenEventKind::Mint),
        transfer(0, 1, 1, Some(G_RECEIVER), None, TokenEventKind::Clawback),
    ];
    let rows = build_value_flow_rows(64_259_660, &txs, &[], &[], &t)
        .unwrap()
        .transfers;
    assert_eq!(
        (
            rows[0].from_id,
            rows[0].from_kind.as_str(),
            rows[0].verb.as_str()
        ),
        (None, "", "mint")
    );
    assert_eq!(
        (
            rows[1].to_id,
            rows[1].to_kind.as_str(),
            rows[1].verb.as_str()
        ),
        (None, "", "clawback")
    );
}

#[test]
fn memo_is_one_row_per_transaction_not_per_transfer() {
    let txs = [tx(Some(("text", "pspb:5721732")), None)];
    let t = [
        transfer(
            0,
            0,
            0,
            Some(G_SENDER),
            Some(G_RECEIVER),
            TokenEventKind::Transfer,
        ),
        transfer(
            0,
            1,
            1,
            Some(G_SENDER),
            Some(G_RECEIVER),
            TokenEventKind::Transfer,
        ),
    ];
    let out = build_value_flow_rows(64_259_660, &txs, &[], &[], &t).unwrap();
    assert_eq!(out.transfers.len(), 2);
    assert_eq!(
        out.memos,
        vec![TransactionMemoRow {
            ledger_sequence: 64_259_660,
            application_order: 1,
            memo_type: "text".into(),
            memo: "pspb:5721732".into(),
        }]
    );
}

#[test]
fn event_ops_cover_per_op_events_only() {
    let txs = [tx(None, None)];
    let evs = [(
        TX.to_string(),
        vec![
            event(EventSource::TxLevel, 0, None),
            event(EventSource::PerOp, 1, Some((0, 0))),
            event(EventSource::PerOp, 2, Some((1, 0))),
            event(EventSource::Diagnostic, 3, None),
        ],
    )];
    let out = build_value_flow_rows(64_259_660, &txs, &[], &evs, &[]).unwrap();
    assert_eq!(
        out.event_ops,
        vec![
            SorobanEventOpRow {
                ledger_sequence: 64_259_660,
                application_order: 1,
                event_index: 1,
                op_index: 0,
                event_pos_in_op: 0
            },
            SorobanEventOpRow {
                ledger_sequence: 64_259_660,
                application_order: 1,
                event_index: 2,
                op_index: 1,
                event_pos_in_op: 0
            },
        ]
    );
}

#[test]
fn a_transfer_for_an_unknown_transaction_is_a_staging_error_not_a_dropped_row() {
    let t = [transfer(
        0,
        0,
        0,
        Some(G_SENDER),
        Some(G_RECEIVER),
        TokenEventKind::Transfer,
    )];
    let err = build_value_flow_rows(64_259_660, &[], &[], &[], &t).unwrap_err();
    assert!(matches!(err, SchemaError::Staging(_)));
}
