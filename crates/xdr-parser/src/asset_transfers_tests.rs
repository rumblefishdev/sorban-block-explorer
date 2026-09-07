use super::*;
use crate::sac::{MAINNET_PASSPHRASE, derive_sac_strkey, network_id};
use domain::ContractEventType;
use serde_json::json;

const G1: &str = "GARNRDKOUGVQ6FMJLL5RPDU6NG36LZHTEOIAWZPNOKFNKNKCOJEC3WMZ";
const G2: &str = "GD5SL5RIC5STHGDDOJGSIIZHZZQPA4HIYFEPIQ3FF7M3H2F5VQ7K2FTL";
/// The KALE issuer from the pre-P23 mint ledgers in the research note.
const KALE_ISSUER: &str = "GBDVX4VELCDSQ54KQJYTNHXAHFLBCA77ZY2USQBM4CSHTTV7DME7KALE";
/// Any contract that is NOT a SAC.
const OTHER_CONTRACT: &str = "CDL74RF5BLYR2YBLCCI7F5FB6TPSCLKEJUBSD2RSVWZ4YHF3VMFAIGWA";

fn net() -> [u8; 32] {
    network_id(MAINNET_PASSPHRASE)
}

fn sym(v: &str) -> Value {
    json!({ "type": "sym", "value": v })
}
fn addr(v: &str) -> Value {
    json!({ "type": "address", "value": v })
}
fn string(v: &str) -> Value {
    json!({ "type": "string", "value": v })
}
fn i128v(v: &str) -> Value {
    json!({ "type": "i128", "value": v })
}
fn map(entries: Vec<(&str, Value)>) -> Value {
    json!({
        "type": "map",
        "value": entries.into_iter().map(|(k, v)| json!({ "key": sym(k), "value": v })).collect::<Vec<_>>()
    })
}

fn event(
    emitter: Option<&str>,
    topics: Vec<Value>,
    data: Value,
    source: EventSource,
    op: Option<(u32, u32)>,
) -> ExtractedEvent {
    ExtractedEvent {
        transaction_hash: "ab".repeat(32),
        event_type: ContractEventType::Contract,
        source,
        contract_id: emitter.map(str::to_string),
        topics: Value::Array(topics),
        data,
        event_index: 3,
        op_index: op.map(|o| o.0),
        event_pos_in_op: op.map(|o| o.1),
        stage: None,
        ledger_sequence: 64_259_660,
        created_at: 0,
    }
}

// ---- token_event_amount ------------------------------------------------

#[test]
fn sep50_unsigned_token_ids_are_not_amounts() {
    use crate::scval::scval_to_typed_json;
    use stellar_xdr::{ScVal, UInt128Parts, UInt256Parts};
    for raw in [
        ScVal::U32(123),
        ScVal::U64(123),
        ScVal::U128(UInt128Parts { hi: 0, lo: 123 }),
        ScVal::U128(UInt128Parts {
            hi: u64::MAX,
            lo: u64::MAX,
        }),
        ScVal::U256(UInt256Parts {
            hi_hi: u64::MAX,
            hi_lo: 0,
            lo_hi: 0,
            lo_lo: 123,
        }),
    ] {
        let data = scval_to_typed_json(&raw);
        assert_eq!(
            token_event_amount(&data),
            TokenAmount::NonFungible,
            "{data}"
        );
    }
}

#[test]
fn contradictory_or_invalid_token_id_maps_are_rejected() {
    for data in [
        map(vec![
            ("amount", i128v("1000000000")),
            ("token_id", json!({"type":"u32","value":123})),
        ]),
        map(vec![("token_id", json!({"type":"void","value":null}))]),
        map(vec![("amount", json!({"type":"u128","value":"-1"}))]),
        map(vec![(
            "amount",
            json!({"type":"u128","value":u128::MAX.to_string()}),
        )]),
        json!({"type":"u32","value":4294967296u64}),
        json!({"type":"u64","value":-1}),
        json!({"type":"u128","value":"-1"}),
    ] {
        assert_eq!(token_event_amount(&data), TokenAmount::Unrecognised);
    }
}

#[test]
fn sep50_mint_cannot_credit_the_token_number_as_an_amount() {
    let ev = event(
        Some(OTHER_CONTRACT),
        vec![sym("mint"), addr(G2)],
        crate::scval::scval_to_typed_json(&stellar_xdr::ScVal::U128(stellar_xdr::UInt128Parts {
            hi: 0,
            lo: 1_000_000_000,
        })),
        EventSource::PerOp,
        Some((0, 0)),
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.rejects.is_empty());
    assert_eq!(out.transfers.len(), 1);
    assert_eq!(out.transfers[0].to.as_deref(), Some(G2));
    assert_eq!(out.transfers[0].amount, None);
}

#[test]
fn scalar_i128_is_the_amount() {
    assert_eq!(
        token_event_amount(&i128v("5033540")),
        TokenAmount::Fungible(5_033_540)
    );
    assert_eq!(
        token_event_amount(&json!({ "type": "u128", "value": "7" })),
        TokenAmount::NonFungible
    );
}

#[test]
fn map_amount_is_read_by_key_not_position() {
    // The measured muxed/memo shape — `to_muxed_id` first, amount second.
    let data = map(vec![
        ("to_muxed_id", string("pspb:5721732")),
        ("amount", i128v("182000000")),
    ]);
    assert_eq!(
        token_event_amount(&data),
        TokenAmount::Fungible(182_000_000)
    );
}

#[test]
fn lp_position_mint_amount_is_the_position_not_a_component() {
    let data = map(vec![
        ("amount0", json!({ "type": "u128", "value": "10" })),
        ("amount1", json!({ "type": "u128", "value": "20" })),
        ("amount", json!({ "type": "u128", "value": "3" })),
        ("owner", addr(G1)),
    ]);
    assert_eq!(token_event_amount(&data), TokenAmount::Fungible(3));
}

#[test]
fn token_id_means_non_fungible() {
    let data = map(vec![("token_id", json!({ "type": "u32", "value": 942 }))]);
    assert_eq!(token_event_amount(&data), TokenAmount::NonFungible);
}

#[test]
fn protocol_annotations_and_odd_scalars_are_unrecognised() {
    let restated_mint = map(vec![
        (
            "mint_amount",
            json!({ "type": "u128", "value": "6000000000" }),
        ),
        ("mint_tokens", json!({ "type": "u128", "value": "1" })),
    ]);
    assert_eq!(
        token_event_amount(&restated_mint),
        TokenAmount::Unrecognised
    );
    assert_eq!(
        token_event_amount(&json!({ "type": "void" })),
        TokenAmount::Unrecognised
    );
    assert_eq!(
        token_event_amount(&json!({ "type": "u64", "value": 5 })),
        TokenAmount::NonFungible
    );
    // Token IDs are not restricted by the signed amount column's range.
    assert_eq!(
        token_event_amount(
            &json!({ "type": "u128", "value": "340282366920938463463374607431768211455" })
        ),
        TokenAmount::NonFungible
    );
}

// ---- extract_asset_transfers -------------------------------------------

#[test]
fn native_transfer_from_the_real_xlm_sac_is_a_row() {
    let xlm_sac = derive_sac_strkey("", "", &net()).expect("native SAC derives");
    let ev = event(
        Some(&xlm_sac),
        vec![sym("transfer"), addr(G1), addr(G2), string("native")],
        map(vec![
            ("amount", i128v("10000")),
            (
                "to_muxed_id",
                json!({ "type": "u64", "value": 3539365402u64 }),
            ),
        ]),
        EventSource::PerOp,
        Some((0, 0)),
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.rejects.is_empty(), "{:?}", out.rejects);
    assert_eq!(
        out.transfers,
        vec![ExtractedAssetTransfer {
            transaction_hash: "ab".repeat(32),
            event_index: 3,
            op_index: 0,
            event_pos_in_op: 0,
            kind: TokenEventKind::Transfer,
            from: Some(G1.into()),
            to: Some(G2.into()),
            asset: EventAsset::Native,
            emitter: xlm_sac,
            amount: Some(10_000),
        }]
    );
}

#[test]
fn credit_asset_from_its_own_sac_passes_the_gate() {
    let kale_sac = derive_sac_strkey("KALE", KALE_ISSUER, &net()).expect("SAC derives");
    let ev = event(
        Some(&kale_sac),
        vec![
            sym("mint"),
            addr(G2),
            string(&format!("KALE:{KALE_ISSUER}")),
        ],
        i128v("39298327"),
        EventSource::PerOp,
        Some((0, 0)),
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.rejects.is_empty(), "{:?}", out.rejects);
    let t = &out.transfers[0];
    assert_eq!(t.kind, TokenEventKind::Mint);
    assert_eq!(t.from, None);
    assert_eq!(t.to.as_deref(), Some(G2));
    assert_eq!(
        t.asset,
        EventAsset::Credit {
            code: "KALE".into(),
            issuer: KALE_ISSUER.into()
        }
    );
    assert_eq!(t.amount, Some(39_298_327));
}

#[test]
fn a_foreign_contract_claiming_a_labelled_asset_is_rejected_not_stored() {
    // The spoofing shape: OTHER_CONTRACT says it moved KALE.
    let ev = event(
        Some(OTHER_CONTRACT),
        vec![
            sym("transfer"),
            addr(G1),
            addr(G2),
            string(&format!("KALE:{KALE_ISSUER}")),
        ],
        i128v("1"),
        EventSource::PerOp,
        Some((0, 0)),
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.transfers.is_empty());
    assert_eq!(
        out.rejects,
        vec![TransferReject {
            transaction_hash: "ab".repeat(32),
            event_index: 3,
            emitter: Some(OTHER_CONTRACT.into()),
            kind: RejectKind::EmitterNotSac {
                asset: format!("KALE:{KALE_ISSUER}"),
            },
        }]
    );
}

#[test]
fn bespoke_token_is_its_own_asset_and_needs_no_gate() {
    let ev = event(
        Some(OTHER_CONTRACT),
        vec![sym("transfer"), addr(G1), addr(G2)],
        i128v("500"),
        EventSource::PerOp,
        Some((2, 5)),
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.rejects.is_empty());
    let t = &out.transfers[0];
    assert_eq!(t.asset, EventAsset::Bespoke);
    assert_eq!(t.emitter, OTHER_CONTRACT);
    assert_eq!((t.op_index, t.event_pos_in_op), (2, 5));
}

#[test]
fn non_fungible_movement_is_a_row_with_no_amount() {
    let ev = event(
        Some(OTHER_CONTRACT),
        vec![sym("burn"), addr(G1)],
        map(vec![("token_id", json!({ "type": "u32", "value": 942 }))]),
        EventSource::PerOp,
        Some((0, 1)),
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.rejects.is_empty());
    assert_eq!(out.transfers[0].kind, TokenEventKind::Burn);
    assert_eq!(out.transfers[0].amount, None);
    assert_eq!(out.transfers[0].to, None);
}

#[test]
fn restated_mint_is_rejected_and_counted_never_a_phantom_row() {
    let ev = event(
        Some(OTHER_CONTRACT),
        vec![sym("mint"), addr(G2)],
        map(vec![
            (
                "mint_amount",
                json!({ "type": "u128", "value": "6000000000" }),
            ),
            ("mint_tokens", json!({ "type": "u128", "value": "1" })),
        ]),
        EventSource::PerOp,
        Some((0, 13)),
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.transfers.is_empty());
    assert!(matches!(
        out.rejects.as_slice(),
        [TransferReject { kind: RejectKind::UnrecognisedPayload { verb: TokenEventKind::Mint, data_type }, .. }]
            if data_type == "map"
    ));
}

#[test]
fn diagnostic_and_non_token_events_are_skipped_silently() {
    let diag = event(
        Some(OTHER_CONTRACT),
        vec![sym("transfer"), addr(G1), addr(G2)],
        i128v("500"),
        EventSource::Diagnostic,
        None,
    );
    let fn_call = event(
        Some(OTHER_CONTRACT),
        vec![sym("fn_call"), addr(G1)],
        json!({ "type": "void" }),
        EventSource::PerOp,
        Some((0, 0)),
    );
    let out = extract_asset_transfers(&[diag, fn_call], &net());
    assert!(out.transfers.is_empty());
    assert!(out.rejects.is_empty());
}

#[test]
fn a_token_verb_outside_an_operation_is_a_reject() {
    let ev = event(
        Some(OTHER_CONTRACT),
        vec![sym("transfer"), addr(G1), addr(G2)],
        i128v("500"),
        EventSource::TxLevel,
        None,
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.transfers.is_empty());
    assert!(matches!(
        out.rejects.as_slice(),
        [TransferReject {
            event_index: 3,
            kind: RejectKind::NoOperation,
            ..
        }]
    ));
}

// ---- deep-review fixes (2026-09-07) --------------------------------------

/// The SEP-41 / `soroban-token-sdk` mint shape `[mint, admin, to]`, measured
/// on mainnet (3 169 events in ledgers 64 000 000–64 100 000): the emitter is
/// its own admin and the recipient is the SECOND address. Before the fix the
/// row credited the admin contract and the recipient vanished.
#[test]
fn sep41_mint_with_admin_credits_the_recipient_not_the_admin() {
    let ev = event(
        Some(OTHER_CONTRACT),
        vec![sym("mint"), addr(OTHER_CONTRACT), addr(G1)],
        i128v("11368693905"),
        EventSource::PerOp,
        Some((0, 3)),
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.rejects.is_empty());
    let [t] = out.transfers.as_slice() else {
        panic!("expected one transfer, got {:?}", out.transfers);
    };
    assert_eq!(t.kind, TokenEventKind::Mint);
    assert_eq!(t.from, None);
    assert_eq!(t.to.as_deref(), Some(G1));
    assert_eq!(t.asset, EventAsset::Bespoke);
    assert_eq!(t.emitter, OTHER_CONTRACT);
    assert_eq!(t.amount, Some(11_368_693_905));
}

/// A token verb in a topic shape the decoder does not know (the 1-topic
/// `mint` of a concentrated-liquidity position contract) is a counted reject,
/// never silence — the module doc promises exactly that.
#[test]
fn a_token_verb_in_an_unknown_topic_shape_is_a_reject_not_silence() {
    let ev = event(
        Some(OTHER_CONTRACT),
        vec![sym("mint")],
        map(vec![("amount", i128v("5")), ("amount0", i128v("1"))]),
        EventSource::PerOp,
        Some((0, 0)),
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.transfers.is_empty());
    assert!(matches!(
        out.rejects.as_slice(),
        [TransferReject {
            kind: RejectKind::UnrecognisedTopics {
                verb: TokenEventKind::Mint,
                topic_count: 1
            },
            ..
        }]
    ));
    assert_eq!(out.reject_counts().unrecognised_topics, 1);
    assert_eq!(out.reject_counts().total(), 1);
}

/// No emitting contract → no asset identity → a reject, not a row with
/// `hash64("")` as its asset.
#[test]
fn a_token_verb_without_an_emitter_is_a_reject() {
    let ev = event(
        None,
        vec![sym("transfer"), addr(G1), addr(G2)],
        i128v("500"),
        EventSource::PerOp,
        Some((0, 0)),
    );
    let out = extract_asset_transfers(&[ev], &net());
    assert!(out.transfers.is_empty());
    assert!(matches!(
        out.rejects.as_slice(),
        [TransferReject {
            event_index: 3,
            emitter: None,
            kind: RejectKind::NoEmitter,
            ..
        }]
    ));
    assert_eq!(out.reject_counts().no_emitter, 1);
}
