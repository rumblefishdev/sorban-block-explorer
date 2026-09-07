//! Cross-cutting tests for the CH writer.
//!
//! Two categories:
//!
//! 1. **Column-order pinning** — `clickhouse::Row::COLUMN_NAMES` per
//!    row struct must match `init.sql` byte-for-byte (RowBinary is
//!    positional; mismatch silently corrupts every row).
//! 2. **Staging behaviour** — `stage::prepare` smoke tests for the
//!    invariants production code depends on (surrogate-id derivation,
//!    FK consistency by integer equality, stub-rowing, Native
//!    singleton, op fold, signature extraction, diagnostic drop).

use super::*;
use clickhouse::Row;
use rows::*;

fn assert_columns<R: Row>(table: &'static str, expected: &[&str]) {
    assert_eq!(
        R::COLUMN_NAMES,
        expected,
        "row struct columns out of sync with init.sql for `{table}`. \
         RowBinary is positional — any drift silently corrupts every row written."
    );
}

#[test]
fn column_order_ledgers() {
    assert_columns::<LedgerRow>(
        "ledgers",
        &[
            "sequence",
            "hash",
            "closed_at",
            "protocol_version",
            "transaction_count",
            "base_fee",
        ],
    );
}

#[test]
fn column_order_wasm_interface_metadata() {
    assert_columns::<WasmInterfaceMetadataRow>(
        "wasm_interface_metadata",
        &["wasm_hash", "metadata"],
    );
}

#[test]
fn column_order_accounts() {
    assert_columns::<AccountRow>(
        "accounts",
        &[
            "id",
            "account_id",
            "first_seen_ledger",
            "last_seen_ledger",
            "sequence_number",
            "home_domain",
        ],
    );
}

#[test]
fn column_order_account_entry_state() {
    assert_columns::<AccountEntryStateRow>(
        "account_entry_state",
        &[
            "account_id",
            "signer_keys",
            "signer_weights",
            "signer_types",
            "master_weight",
            "threshold_low",
            "threshold_med",
            "threshold_high",
            "flags",
            "last_updated_ledger",
        ],
    );
}

#[test]
fn column_order_assets() {
    assert_columns::<AssetRow>(
        "assets",
        &[
            "asset_type",
            "asset_code",
            "issuer_id",
            "contract_id",
            "id", // lore-0331 surrogate (last)
        ],
    );
}

/// ADR 0051 — SAC facet side table. Positional RowBinary must match `init.sql`.
#[test]
fn column_order_asset_sac() {
    assert_columns::<AssetSacRow>(
        "asset_sac",
        &[
            "asset_type",
            "asset_code",
            "issuer_id",
            "contract_id",
            "sac_contract_id",
            "sac_deployed",
        ],
    );
}

/// Task 0231 — off-chain enrichment side table. Keyed like `assets` +
/// enrichment columns + `version`; positional RowBinary must match `init.sql`.
#[test]
fn column_order_asset_enrichment() {
    assert_columns::<AssetEnrichmentRow>(
        "asset_enrichment",
        &[
            "asset_type",
            "asset_code",
            "issuer_id",
            "contract_id",
            "icon_url",
            "name",
            "version",
        ],
    );
}

#[test]
fn column_order_balances() {
    assert_columns::<BalanceRow>(
        "balances",
        &[
            "holder_id",
            "asset_id",
            "amount",
            "last_updated_ledger",
            "closed_at_ledger",
        ],
    );
}

#[test]
fn column_order_soroban_contracts() {
    assert_columns::<SorobanContractRow>(
        "soroban_contracts",
        &[
            "id",
            "contract_id",
            "wasm_hash",
            "wasm_uploaded_at_ledger",
            "deployer_id",
            "deployed_at_ledger",
            "contract_type",
            "is_sac",
        ],
    );
}

#[test]
fn column_order_nfts() {
    assert_columns::<NftRow>(
        "nfts",
        &[
            "contract_id",
            "token_id",
            "collection_name",
            "name",
            "media_url",
            "minted_at_ledger",
            "current_owner_id",
            "current_owner_ledger",
        ],
    );
}

/// Task 0217 / 0220 — quarantine companion to [`NftRow`]. Same shape;
/// `clickhouse::Row::COLUMN_NAMES` ordering must stay byte-for-byte in
/// sync with `init.sql` `nfts_pending` because RowBinary is positional.
#[test]
fn column_order_nfts_pending() {
    assert_columns::<NftPendingRow>(
        "nfts_pending",
        &[
            "contract_id",
            "token_id",
            "collection_name",
            "name",
            "media_url",
            "minted_at_ledger",
            "current_owner_id",
            "current_owner_ledger",
        ],
    );
}

/// Task 0231 — off-chain enrichment side table for NFTs. Keyed like `nfts`
/// + metadata + `version`; positional RowBinary must match `init.sql`.
#[test]
fn column_order_nft_enrichment() {
    assert_columns::<NftEnrichmentRow>(
        "nft_enrichment",
        &[
            "contract_id",
            "token_id",
            "name",
            "media_url",
            "collection_name",
            "version",
        ],
    );
}

#[test]
fn column_order_liquidity_pools() {
    assert_columns::<LiquidityPoolRow>(
        "liquidity_pools",
        &[
            "pool_id",
            "asset_a_type",
            "asset_a_code",
            "asset_a_issuer_id",
            "asset_b_type",
            "asset_b_code",
            "asset_b_issuer_id",
            "fee_bps",
            "last_updated_ledger",
            "pool_kind",
            "legs",
            "deployment_id",
            "pool_type_raw",
        ],
    );
}

#[test]
fn column_order_pool_state_changes() {
    assert_columns::<PoolStateChangeRow>(
        "pool_state_changes",
        &["pool_id", "ledger_sequence", "reserves", "plane_id"],
    );
}

#[test]
fn column_order_pool_instance_state() {
    assert_columns::<PoolInstanceStateRow>(
        "pool_instance_state",
        &[
            "pool_id",
            "plane_id",
            "share_token_id",
            "total_shares",
            "derived_at_ledger",
        ],
    );
}

#[test]
fn column_order_lp_positions() {
    assert_columns::<LpPositionRow>(
        "lp_positions",
        &[
            "pool_id",
            "account_id",
            "shares",
            "first_deposit_ledger",
            "last_updated_ledger",
            "closed_at_ledger",
        ],
    );
}

#[test]
fn column_order_transactions() {
    assert_columns::<TransactionRow>(
        "transactions",
        &[
            "id",
            "hash",
            "ledger_sequence",
            "application_order",
            "source_id",
            "fee_charged",
            "inner_tx_hash",
            "successful",
            "operation_count",
            "has_soroban",
            "parse_error",
        ],
    );
}

#[test]
fn column_order_transaction_hash_index() {
    assert_columns::<TransactionHashIndexRow>(
        "transaction_hash_index",
        &["hash", "ledger_sequence"],
    );
}

#[test]
fn column_order_operations_appearances() {
    assert_columns::<OperationAppearanceRow>(
        "operations_appearances",
        &[
            "transaction_id",
            "application_order",
            "type",
            "source_id",
            "destination_id",
            "contract_id",
            "asset_code",
            "asset_issuer_id",
            "pool_ids",
            "amount",
            "ledger_sequence",
        ],
    );
}

#[test]
fn column_order_transaction_participants() {
    assert_columns::<TransactionParticipantRow>(
        "transaction_participants",
        &["account_id", "ledger_sequence", "transaction_id"],
    );
}

#[test]
fn column_order_operation_asset_appearances() {
    assert_columns::<OperationAssetAppearanceRow>(
        "operation_asset_appearances",
        &["asset_id", "ledger_sequence", "transaction_id"],
    );
}

#[test]
fn column_order_operation_pools() {
    assert_columns::<OperationPoolRow>(
        "operation_pools",
        &["pool_id", "ledger_sequence", "transaction_id"],
    );
}

#[test]
fn column_order_soroban_events() {
    assert_columns::<SorobanEventRow>(
        "soroban_events",
        &[
            "contract_id",
            "transaction_id",
            "ledger_sequence",
            "event_index",
            "event_type",
            "signature",
            "topics_xdr",
            "data_xdr",
        ],
    );
}

#[test]
fn column_order_soroban_invocations_appearances() {
    assert_columns::<SorobanInvocationAppearanceRow>(
        "soroban_invocations_appearances",
        &[
            "contract_id",
            "transaction_id",
            "ledger_sequence",
            "caller_id",
            "caller_contract_id",
            "amount",
        ],
    );
}

#[test]
fn column_order_nft_ownership() {
    assert_columns::<NftOwnershipRow>(
        "nft_ownership",
        &[
            "contract_id",
            "token_id",
            "ledger_sequence",
            "event_order",
            "transaction_id",
            "owner_id",
            "event_type",
        ],
    );
}

/// Task 0217 / 0220 — quarantine companion to [`NftOwnershipRow`].
/// Column order must stay byte-for-byte in sync with `init.sql`
/// `nft_ownership_pending` because RowBinary is positional.
#[test]
fn column_order_nft_ownership_pending() {
    assert_columns::<NftOwnershipPendingRow>(
        "nft_ownership_pending",
        &[
            "contract_id",
            "token_id",
            "ledger_sequence",
            "event_order",
            "transaction_id",
            "owner_id",
            "event_type",
        ],
    );
}

#[test]
fn column_order_liquidity_pool_snapshots() {
    assert_columns::<LiquidityPoolSnapshotRow>(
        "liquidity_pool_snapshots",
        &[
            "pool_id",
            "ledger_sequence",
            "reserve_a",
            "reserve_b",
            "total_shares",
            "gross_volume_a",
        ],
    );
}

// ---------------------------------------------------------------------------
// Staging smoke
// ---------------------------------------------------------------------------

use domain::{AssetFamily, ContractEventType, ContractType, OperationType};
use xdr_parser::types::{
    EventSource, ExtractedContractDeployment, ExtractedEvent, ExtractedLedger, ExtractedOperation,
    ExtractedTransaction,
};

fn synthetic_ledger() -> ExtractedLedger {
    ExtractedLedger {
        sequence: 10,
        hash: "00".repeat(32),
        closed_at: 1_700_000_000,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    }
}

fn synthetic_tx(hash_seed: u8) -> ExtractedTransaction {
    let mut bytes = vec![0u8; 32];
    bytes[31] = hash_seed;
    ExtractedTransaction {
        hash: hex::encode(&bytes),
        inner_tx_hash: None,
        ledger_sequence: 10,
        source_account: "G".to_string() + &"A".repeat(55),
        fee_source: None,
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".into(),
        envelope_xdr: String::new(),
        result_xdr: String::new(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: 1_700_000_000,
        parse_error: false,
        ledger_deltas: vec![],
    }
}

#[test]
fn prepare_empty_inputs_yields_ledger_and_native_asset() {
    let ledger = synthetic_ledger();
    let staged = stage::prepare(
        &ledger,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");
    assert_eq!(staged.ledger_rows.len(), 1);
    assert_eq!(staged.transaction_rows.len(), 0);
    assert_eq!(staged.account_rows.len(), 0);
    assert_eq!(staged.event_rows.len(), 0);
    // Native XLM singleton — schema-level concern, writer stages it.
    assert_eq!(staged.asset_rows.len(), 1);
    let native = &staged.asset_rows[0];
    assert_eq!(native.asset_type, AssetFamily::Native as i16);
    assert_eq!(native.asset_code, "");
    assert_eq!(native.issuer_id, 0);
    assert_eq!(native.contract_id, 0);
}

/// FK consistency by Int64 equality: `transactions.source_id` is the
/// same i64 as `accounts.id` for the same StrKey. JOIN by raw integer
/// equality.
#[test]
fn prepare_surrogate_id_fk_consistency() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x10);
    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.account_rows.len(), 1);
    assert_eq!(staged.transaction_rows.len(), 1);
    assert_eq!(staged.hash_index_rows.len(), 1);
    assert_eq!(staged.participant_rows.len(), 1);

    let tx_row = &staged.transaction_rows[0];
    let acc_row = &staged.account_rows[0];
    let part_row = &staged.participant_rows[0];

    // FK = derived i64 via ids::*. Cross-table consistency by integer
    // equality.
    assert_eq!(tx_row.source_id, acc_row.id);
    assert_eq!(part_row.account_id, acc_row.id);
    assert_eq!(part_row.transaction_id, tx_row.id);

    // tx surrogate id derived from same hash bytes as `hash` column.
    assert_eq!(tx_row.id, ids::transaction_id(&tx_row.hash));
}

/// Fee-bump: both the outer and the inner tx hash are indexed to the same
/// ledger, so a lookup by either resolves to the fee-bump (task 0375).
#[test]
fn prepare_fee_bump_indexes_inner_hash() {
    let ledger = synthetic_ledger();
    let mut tx = synthetic_tx(0x10);
    let mut inner = vec![0u8; 32];
    inner[31] = 0x20;
    tx.inner_tx_hash = Some(hex::encode(&inner));

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    // Two index rows: outer + inner, both → the tx's ledger.
    assert_eq!(staged.hash_index_rows.len(), 2);
    let seq = staged.transaction_rows[0].ledger_sequence;
    let mut outer = [0u8; 32];
    outer[31] = 0x10;
    assert!(
        staged
            .hash_index_rows
            .iter()
            .any(|r| r.hash == outer && r.ledger_sequence == seq)
    );
    let mut inner_bytes = [0u8; 32];
    inner_bytes[31] = 0x20;
    assert!(
        staged
            .hash_index_rows
            .iter()
            .any(|r| r.hash == inner_bytes && r.ledger_sequence == seq)
    );
}

#[test]
fn prepare_extracts_signature_from_first_symbol_topic() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x60);
    let contract = "C".to_string() + &"F".repeat(55);

    let make = |topics: serde_json::Value| ExtractedEvent {
        transaction_hash: tx.hash.clone(),
        event_type: ContractEventType::Contract,
        source: EventSource::TxLevel,
        contract_id: Some(contract.clone()),
        topics,
        data: serde_json::json!({}),
        event_index: 0,
        op_index: None,
        stage: None,
        ledger_sequence: 10,
        created_at: 1_700_000_000,
    };
    let events = vec![(
        tx.hash.clone(),
        vec![
            make(serde_json::json!([
                {"type": "sym", "value": "transfer"},
                {"type": "address", "value": "G..."}
            ])),
            make(serde_json::json!([
                {"type": "sym", "value": "fee"},
                {"type": "address", "value": "G..."}
            ])),
            make(serde_json::json!([
                {"type": "address", "value": "G..."}
            ])),
        ],
    )];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &events,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.event_rows.len(), 3);
    let sigs: Vec<Option<String>> = staged
        .event_rows
        .iter()
        .map(|r| r.signature.clone())
        .collect();
    assert!(sigs.contains(&Some("transfer".to_string())));
    assert!(sigs.contains(&Some("fee".to_string())));
    assert!(sigs.contains(&None));
}

#[test]
fn prepare_drops_diagnostic_events_and_orphans() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x20);
    let contract = "C".to_string() + &"D".repeat(55);
    let make = |contract_id: Option<String>, source: EventSource| ExtractedEvent {
        transaction_hash: tx.hash.clone(),
        event_type: ContractEventType::Contract,
        source,
        contract_id,
        topics: serde_json::json!([{"type": "sym", "value": "transfer"}]),
        data: serde_json::json!({}),
        event_index: 0,
        op_index: None,
        stage: None,
        ledger_sequence: 10,
        created_at: 1_700_000_000,
    };
    let events = vec![(
        tx.hash.clone(),
        vec![
            make(Some(contract.clone()), EventSource::TxLevel),
            make(Some(contract.clone()), EventSource::Diagnostic),
            make(None, EventSource::TxLevel),
        ],
    )];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &events,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(
        staged.event_rows.len(),
        1,
        "expected diagnostic + orphan to be filtered, got {}",
        staged.event_rows.len()
    );
    assert_eq!(
        staged.event_rows[0].contract_id,
        ids::contract_id(&contract)
    );
}

#[test]
fn prepare_folds_identical_operations() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x30);
    let dest = "G".to_string() + &"E".repeat(55);
    let make_op = |idx: u32| ExtractedOperation {
        transaction_hash: tx.hash.clone(),
        operation_index: idx,
        op_type: OperationType::Payment,
        source_account: None,
        asset_appearances: vec![],
        counterparties: vec![],
        details: serde_json::json!({
            "destination": dest,
            "asset": "native",
        }),
    };
    let ops = vec![(tx.hash.clone(), vec![make_op(1), make_op(2)])];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.op_rows.len(), 1);
    let op_row = &staged.op_rows[0];
    assert_eq!(op_row.amount, 2);
    assert_eq!(op_row.application_order, 1);
    assert_eq!(op_row.op_type, OperationType::Payment as i16);
    assert_eq!(op_row.destination_id, Some(ids::account_id(&dest)));
}

#[test]
fn prepare_registers_fee_bump_fee_source_as_participant() {
    // Task 0359 K2-4: the fee-bump payer funds the fee but runs no ops and is
    // not the inner source — it must still land in transaction_participants.
    let ledger = synthetic_ledger();
    let mut tx = synthetic_tx(0x64);
    let payer = "G".to_string() + &"P".repeat(55);
    tx.fee_source = Some(payer.clone());

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert!(
        staged
            .participant_rows
            .iter()
            .any(|r| r.account_id == ids::account_id(&payer)),
        "fee-bump payer registered as participant"
    );
    assert!(
        staged
            .account_rows
            .iter()
            .any(|a| a.id == ids::account_id(&payer)),
        "payer gets an accounts stub"
    );
}

#[test]
fn prepare_registers_op_counterparties_as_participants() {
    // Task 0359 F-C (K1-5): a parser-emitted counterparty (here a crossed-offer
    // seller) lands in transaction_participants and gets an accounts stub — a
    // role the string-`details` extraction dropped.
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x63);
    let seller = "G".to_string() + &"S".repeat(55);
    let op = ExtractedOperation {
        transaction_hash: tx.hash.clone(),
        operation_index: 1,
        op_type: OperationType::ManageBuyOffer,
        source_account: None,
        asset_appearances: vec![],
        counterparties: vec![seller.clone()],
        details: serde_json::json!({ "selling": "native", "buying": "native" }),
    };
    let ops = vec![(tx.hash.clone(), vec![op])];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert!(
        staged
            .participant_rows
            .iter()
            .any(|r| r.account_id == ids::account_id(&seller)),
        "crossed-offer seller registered as tx participant"
    );
    assert!(
        staged
            .account_rows
            .iter()
            .any(|a| a.id == ids::account_id(&seller)),
        "seller gets an accounts stub"
    );
}

#[test]
fn prepare_stages_operation_asset_appearances() {
    use xdr_parser::asset_appearances::AssetRef;
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x35);
    let issuer = "G".to_string() + &"I".repeat(55);
    // A sell offer: ZERO assets in the legacy slot, two appearances here — native
    // must key as the FIRST-CLASS surrogate, not an empty sentinel.
    let op = ExtractedOperation {
        transaction_hash: tx.hash.clone(),
        operation_index: 1,
        op_type: OperationType::ManageSellOffer,
        source_account: None,
        asset_appearances: vec![
            AssetRef::Native,
            AssetRef::Credit {
                code: "USDC".into(),
                issuer: issuer.clone(),
            },
        ],
        counterparties: vec![],
        details: serde_json::json!({ "selling": "native", "buying": format!("USDC:{issuer}") }),
    };
    let ops = vec![(tx.hash.clone(), vec![op])];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.op_asset_rows.len(), 2);
    // Native = ids::asset_id(0,"",0,0) — the golden-pinned first-class key.
    assert_eq!(staged.op_asset_rows[0].asset_id, ids::asset_id(0, "", 0, 0));
    // Classic credit hashes code:issuer_surrogate (issuer StrKey hashed first).
    assert_eq!(
        staged.op_asset_rows[1].asset_id,
        ids::asset_id(1, "USDC", ids::account_id(&issuer), 0)
    );
    // Same tx as the legacy fold row — join-back key intact.
    assert_eq!(
        staged.op_asset_rows[0].transaction_id,
        staged.op_rows[0].transaction_id
    );
    // Task 0359 decision 1c: the credit-leg issuer is NOT a tx participant. The
    // asset's activity lives on its asset page (`op_asset_rows` above); flooding
    // the issuer's participant list with every tx touching its asset would be
    // redundant with the asset index.
    assert!(
        !staged
            .participant_rows
            .iter()
            .any(|r| r.account_id == ids::account_id(&issuer)),
        "asset issuer must NOT be registered as a tx participant (decision 1c)"
    );
}

#[test]
fn op_asset_appearances_dedup_same_asset_across_ops_in_one_tx() {
    use xdr_parser::asset_appearances::AssetRef;
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x36);
    let issuer = "G".to_string() + &"I".repeat(55);
    // TWO sell-offer ops in ONE tx, each touching {native, USDC} = 4 appearances.
    // Per-tx dedup collapses to 2 rows, not 4 (PR #6).
    let mk = |idx: u32| ExtractedOperation {
        transaction_hash: tx.hash.clone(),
        operation_index: idx,
        op_type: OperationType::ManageSellOffer,
        source_account: None,
        asset_appearances: vec![
            AssetRef::Native,
            AssetRef::Credit {
                code: "USDC".into(),
                issuer: issuer.clone(),
            },
        ],
        counterparties: vec![],
        details: serde_json::json!({ "selling": "native", "buying": format!("USDC:{issuer}") }),
    };
    let ops = vec![(tx.hash.clone(), vec![mk(1), mk(2)])];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.op_asset_rows.len(), 2);
    let mut ids_seen: Vec<i64> = staged.op_asset_rows.iter().map(|r| r.asset_id).collect();
    ids_seen.sort_unstable();
    let mut want = vec![
        ids::asset_id(0, "", 0, 0),
        ids::asset_id(1, "USDC", ids::account_id(&issuer), 0),
    ];
    want.sort_unstable();
    assert_eq!(ids_seen, want);
}

#[test]
fn prepare_path_payment_pool_ids_split_fold_and_sort() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x31);
    let dest = "G".to_string() + &"E".repeat(55);
    let pool_a = "11".repeat(32);
    let pool_b = "22".repeat(32);
    let make_op = |idx: u32, pools: Vec<&str>| ExtractedOperation {
        transaction_hash: tx.hash.clone(),
        operation_index: idx,
        op_type: OperationType::PathPaymentStrictSend,
        source_account: None,
        asset_appearances: vec![],
        counterparties: vec![],
        details: serde_json::json!({
            "destination": dest,
            "destAsset": "native",
            "poolIds": pools,
        }),
    };
    // op1 crosses B then A (deliberately unsorted), op2 crosses only A —
    // different pool sets refine the fold identity → two rows, not one.
    let ops = vec![(
        tx.hash.clone(),
        vec![
            make_op(1, vec![&pool_b, &pool_a]),
            make_op(2, vec![&pool_a]),
        ],
    )];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.op_rows.len(), 2, "distinct pool sets must not fold");
    let mut rows = staged.op_rows.clone();
    rows.sort_by_key(|r| r.application_order);
    assert_eq!(rows[0].application_order, 1);
    assert_eq!(rows[0].amount, 1);
    assert_eq!(
        rows[0].pool_ids,
        vec![[0x11u8; 32], [0x22u8; 32]],
        "canonical sorted order regardless of crossing order"
    );
    assert_eq!(rows[1].application_order, 2);
    assert_eq!(rows[1].pool_ids, vec![[0x11u8; 32]]);
}

#[test]
fn prepare_sets_gross_volume_a_on_traded_pool_snapshot() {
    // 0261/0266: live ingest now derives gross_volume_a from claimed atoms and
    // stamps it onto the traded pool's snapshot row; an untraded pool stays None.
    let ledger = synthetic_ledger();
    let seq = ledger.sequence;
    let tx = synthetic_tx(0x33);
    let traded = "aa".repeat(32);
    let quiet = "bb".repeat(32);
    let op = ExtractedOperation {
        transaction_hash: tx.hash.clone(),
        operation_index: 1,
        op_type: OperationType::PathPaymentStrictSend,
        source_account: None,
        asset_appearances: vec![],
        counterparties: vec![],
        details: serde_json::json!({
            "poolIds": [traded],
            "claimedAtoms": [
                { "poolId": traded, "amountA": 500i64 },
                { "poolId": traded, "amountA": 300i64 },
            ],
        }),
    };
    let ops = vec![(tx.hash.clone(), vec![op])];
    let snap = |pool: &str| xdr_parser::types::ExtractedLiquidityPoolSnapshot {
        pool_id: pool.to_string(),
        ledger_sequence: seq,
        created_at: 0,
        reserves: serde_json::json!({ "a": 1_000i64, "b": 2_000i64 }),
        total_shares: "0".to_string(),
    };
    let snaps = vec![snap(&traded), snap(&quiet)];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &snaps,
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    let traded_row = staged
        .snapshot_rows
        .iter()
        .find(|r| r.pool_id == [0xAAu8; 32])
        .expect("traded snapshot row");
    assert_eq!(
        traded_row.gross_volume_a,
        Some(800),
        "live prepare must sum claimed-atom amountA per pool"
    );
    let quiet_row = staged
        .snapshot_rows
        .iter()
        .find(|r| r.pool_id == [0xBBu8; 32])
        .expect("quiet snapshot row");
    assert_eq!(
        quiet_row.gross_volume_a, None,
        "untraded pool keeps gross_volume_a = None"
    );
}

#[test]
fn prepare_lp_deposit_single_element_pool_ids() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x32);
    let pool = "ab".repeat(32);
    let op = ExtractedOperation {
        transaction_hash: tx.hash.clone(),
        operation_index: 1,
        op_type: OperationType::LiquidityPoolDeposit,
        source_account: None,
        asset_appearances: vec![],
        counterparties: vec![],
        details: serde_json::json!({ "liquidityPoolId": pool }),
    };
    let ops = vec![(tx.hash.clone(), vec![op])];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.op_rows.len(), 1);
    assert_eq!(staged.op_rows[0].pool_ids, vec![[0xABu8; 32]]);
    // task 0365: the same crossing fans out into operation_pools (pool, tx).
    assert_eq!(staged.op_pool_rows.len(), 1);
    assert_eq!(staged.op_pool_rows[0].pool_id, [0xABu8; 32]);
    assert_eq!(
        staged.op_pool_rows[0].transaction_id,
        staged.op_rows[0].transaction_id
    );
}

#[test]
fn prepare_offer_op_pool_ids_from_details() {
    // An offer op crossing an LP carries poolIds in details (parser reads the
    // ManageOfferSuccessResult claim atoms — task 0261/0266 generic extractor);
    // the CH fold must tag pool_ids for it, not just path payments.
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x33);
    let pool = "cd".repeat(32);
    let op = ExtractedOperation {
        transaction_hash: tx.hash.clone(),
        operation_index: 1,
        op_type: OperationType::ManageBuyOffer,
        source_account: None,
        asset_appearances: vec![],
        counterparties: vec![],
        details: serde_json::json!({
            "offerId": 0,
            "poolIds": [pool],
        }),
    };
    let ops = vec![(tx.hash.clone(), vec![op])];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.op_rows.len(), 1);
    assert_eq!(staged.op_rows[0].pool_ids, vec![[0xCDu8; 32]]);
}

#[test]
fn op_pool_rows_dedup_same_pool_across_ops_in_one_tx() {
    // Two ops in one tx crossing the SAME pool → one (pool, tx) row (the per-tx
    // dedup, task 0365). The RMT would collapse residuals anyway; deduping at write
    // cuts the backfilled volume up front — the pool twin of the asset fan-out.
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x34);
    let pool = "ef".repeat(32);
    let mk = |idx: u32| ExtractedOperation {
        transaction_hash: tx.hash.clone(),
        operation_index: idx,
        op_type: OperationType::LiquidityPoolDeposit,
        source_account: None,
        asset_appearances: vec![],
        counterparties: vec![],
        details: serde_json::json!({ "liquidityPoolId": pool }),
    };
    let ops = vec![(tx.hash.clone(), vec![mk(1), mk(2)])];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &ops,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.op_pool_rows.len(), 1);
    assert_eq!(staged.op_pool_rows[0].pool_id, [0xEFu8; 32]);
    assert_eq!(
        staged.op_pool_rows[0].transaction_id,
        staged.op_rows[0].transaction_id
    );
}

#[test]
fn prepare_is_deterministic_across_runs() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x40);
    let a = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("first run");
    let b = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("second run");

    assert_eq!(a.transaction_rows[0].id, b.transaction_rows[0].id);
    assert_eq!(a.account_rows[0].id, b.account_rows[0].id);
    assert_eq!(a.ledger_rows[0].sequence, b.ledger_rows[0].sequence);
}

#[test]
fn prepare_emits_stub_soroban_contract_rows_for_referenced_only() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x70);
    let referenced_contract = "C".to_string() + &"E".repeat(55);

    let event = ExtractedEvent {
        transaction_hash: tx.hash.clone(),
        event_type: ContractEventType::Contract,
        source: EventSource::TxLevel,
        contract_id: Some(referenced_contract.clone()),
        topics: serde_json::json!([{"type": "sym", "value": "transfer"}]),
        data: serde_json::json!({}),
        event_index: 0,
        op_index: None,
        stage: None,
        ledger_sequence: 10,
        created_at: 1_700_000_000,
    };

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[(tx.hash.clone(), vec![event])],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.contract_rows.len(), 1);
    let row = &staged.contract_rows[0];
    assert_eq!(row.contract_id, referenced_contract);
    assert_eq!(row.id, ids::contract_id(&referenced_contract));
    assert_eq!(row.wasm_uploaded_at_ledger, 0);
    assert!(row.wasm_hash.is_none());
    assert!(!row.is_sac);

    // FK from event references the same id.
    assert_eq!(staged.event_rows[0].contract_id, row.id);
}

#[test]
fn prepare_does_not_duplicate_when_contract_both_deployed_and_referenced() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x71);
    let contract = "C".to_string() + &"7".repeat(55);

    let dep = ExtractedContractDeployment {
        contract_id: contract.clone(),
        wasm_hash: None,
        deployer_account: None,
        deployed_at_ledger: 10,
        contract_type: ContractType::Other,
        is_sac: false,
        sac_asset: None,
    };
    let event = ExtractedEvent {
        transaction_hash: tx.hash.clone(),
        event_type: ContractEventType::Contract,
        source: EventSource::TxLevel,
        contract_id: Some(contract.clone()),
        topics: serde_json::json!([{"type": "sym", "value": "init"}]),
        data: serde_json::json!({}),
        event_index: 0,
        op_index: None,
        stage: None,
        ledger_sequence: 10,
        created_at: 1_700_000_000,
    };

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[(tx.hash.clone(), vec![event])],
        &[],
        &[],
        std::slice::from_ref(&dep),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.contract_rows.len(), 1);
    let row = &staged.contract_rows[0];
    assert_eq!(row.contract_id, contract);
    assert_eq!(
        row.wasm_uploaded_at_ledger, 10,
        "real deploy wins over stub"
    );
}

#[test]
fn enum_discriminants_lock_in_with_schema() {
    assert_eq!(AssetFamily::Native as i16, 0);
    assert_eq!(AssetFamily::ClassicCredit as i16, 1);
    assert_eq!(ContractType::Token as i16, 0);
    assert_eq!(ContractType::Other as i16, 1);
    assert_eq!(ContractType::Nft as i16, 2);
    assert_eq!(ContractEventType::System as i16, 0);
    assert_eq!(ContractEventType::Contract as i16, 1);
    assert_eq!(ContractEventType::Diagnostic as i16, 2);
    assert_eq!(OperationType::CreateAccount as i16, 0);
    assert_eq!(OperationType::Payment as i16, 1);
}

// ---------------------------------------------------------------------------
// Task 0217 / 0220 — NFT routing into hot / pending / drop buckets
// ---------------------------------------------------------------------------

use domain::NftEventType;
use xdr_parser::SacOverride;
use xdr_parser::types::{
    ContractFunction, ExtractedContractInterface, ExtractedLiquidityPool, ExtractedLpPosition,
    ExtractedNft, ExtractedNftEvent, SacAssetIdentity,
};

fn synthetic_nft(contract: &str, token: &str) -> ExtractedNft {
    ExtractedNft {
        contract_id: contract.to_string(),
        token_id: token.to_string(),
        collection_name: None,
        owner_account: None,
        name: None,
        media_url: None,
        minted_at_ledger: Some(10),
        last_seen_ledger: 10,
        created_at: 1_700_000_000,
    }
}

fn synthetic_nft_event(
    tx_hash: &str,
    contract: &str,
    token: &str,
    event_order: u16,
) -> ExtractedNftEvent {
    ExtractedNftEvent {
        transaction_hash: tx_hash.to_string(),
        contract_id: contract.to_string(),
        token_id: token.to_string(),
        event_type: NftEventType::Mint,
        owner_account: None,
        event_order,
        ledger_sequence: 10,
        created_at: 1_700_000_000,
    }
}

/// Build a minimal `ExtractedContractInterface` that the wasm-spec
/// classifier reads as `Nft` (OwnerOf is a discriminator function).
fn nft_classified_interface(wasm_hash_hex: &str) -> ExtractedContractInterface {
    ExtractedContractInterface {
        wasm_hash: wasm_hash_hex.to_string(),
        functions: vec![ContractFunction {
            name: "owner_of".into(),
            doc: String::new(),
            inputs: vec![],
            outputs: vec!["Address".into()],
        }],
        wasm_byte_len: 256,
        upgradeable: false,
    }
}

/// Build a minimal `ExtractedContractInterface` that the wasm-spec
/// classifier reads as `Fungible` (Decimals is a discriminator function).
fn fungible_classified_interface(wasm_hash_hex: &str) -> ExtractedContractInterface {
    ExtractedContractInterface {
        wasm_hash: wasm_hash_hex.to_string(),
        functions: vec![ContractFunction {
            name: "decimals".into(),
            doc: String::new(),
            inputs: vec![],
            outputs: vec!["u32".into()],
        }],
        wasm_byte_len: 256,
        upgradeable: false,
    }
}

/// NFT row whose contract was deployed in the same ledger with a
/// definitive `Nft` wasm-classifier verdict routes into the hot
/// `nft_rows` bucket and produces zero `nft_pending_rows`.
#[test]
fn prepare_routes_nft_classified_contract_to_hot_bucket() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x90);
    let contract = "C".to_string() + &"A".repeat(55);
    let wasm_hex = "11".repeat(32);

    let iface = nft_classified_interface(&wasm_hex);
    let dep = ExtractedContractDeployment {
        contract_id: contract.clone(),
        wasm_hash: Some(wasm_hex.clone()),
        deployer_account: None,
        deployed_at_ledger: 10,
        contract_type: ContractType::Other, // parser default; classifier overrides
        is_sac: false,
        sac_asset: None,
    };
    let nft = synthetic_nft(&contract, "tk1");
    let ev = synthetic_nft_event(&tx.hash, &contract, "tk1", 0);

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        std::slice::from_ref(&iface),
        std::slice::from_ref(&dep),
        &[],
        &[],
        &[],
        &[],
        std::slice::from_ref(&nft),
        std::slice::from_ref(&ev),
        &[],
    )
    .expect("prepare");

    assert_eq!(
        staged.nft_rows.len(),
        1,
        "Nft-classified contract: row in hot bucket"
    );
    assert_eq!(
        staged.nft_pending_rows.len(),
        0,
        "Nft-classified contract: nothing in pending"
    );
    assert_eq!(staged.nft_ownership_rows.len(), 1);
    assert_eq!(staged.nft_ownership_pending_rows.len(), 0);

    // Classifier override visible on the contract row.
    let contract_row = &staged.contract_rows[0];
    assert_eq!(contract_row.contract_id, contract);
    assert_eq!(contract_row.contract_type, Some(ContractType::Nft as i16));
}

// ---------------------------------------------------------------------------
// Task 0283 live G1 — cross-ledger WASM verdict via `prior_wasm_verdicts`
// ---------------------------------------------------------------------------

/// The common Soroban case: the WASM was uploaded in an EARLIER ledger, so it
/// is NOT in this ledger's `contract_interfaces` (the same-ledger map is
/// empty). The writer pre-fetched its `Nft` verdict from
/// `wasm_interface_metadata` and passes it via `prior_wasm_verdicts`. The
/// deploy override must consult that fallback and flip the contract to `Nft`
/// — and because the override runs before NFT routing, this ledger's NFT row
/// routes straight to the hot bucket (no quarantine round-trip).
#[test]
fn prepare_applies_prior_wasm_verdict_when_wasm_uploaded_earlier_ledger() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x9A);
    let contract = "C".to_string() + &"D".repeat(55);
    let wasm_hex = "11".repeat(32);

    // No interface in THIS ledger — the upload happened earlier. The verdict
    // comes only from the pre-fetched cross-ledger map (keyed by raw hash).
    let dep = ExtractedContractDeployment {
        contract_id: contract.clone(),
        wasm_hash: Some(wasm_hex.clone()),
        deployer_account: None,
        deployed_at_ledger: 10,
        contract_type: ContractType::Other, // parser default; prior verdict overrides
        is_sac: false,
        sac_asset: None,
    };
    let nft = synthetic_nft(&contract, "tk1");
    let ev = synthetic_nft_event(&tx.hash, &contract, "tk1", 0);
    let prior: std::collections::HashMap<[u8; 32], ContractType> =
        std::collections::HashMap::from([([0x11u8; 32], ContractType::Nft)]);

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[], // EMPTY — wasm not uploaded this ledger
        contract_deployments: std::slice::from_ref(&dep),
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: std::slice::from_ref(&nft),
        nft_events: std::slice::from_ref(&ev),
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &prior,
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare_with_sac_overrides");

    // G1: contract row flipped to Nft from the cross-ledger verdict.
    let contract_row = &staged.contract_rows[0];
    assert_eq!(contract_row.contract_id, contract);
    assert_eq!(contract_row.contract_type, Some(ContractType::Nft as i16));

    // G9-for-free: the corrected verdict feeds NFT routing in the same pass,
    // so the NFT lands in the hot bucket, not quarantine.
    assert_eq!(staged.nft_rows.len(), 1, "routed to hot, not pending");
    assert_eq!(staged.nft_pending_rows.len(), 0);
}

// ---------------------------------------------------------------------------
// Task 0283 live G2 — assets type-3 row for WASM-classified Soroban fungibles
// ---------------------------------------------------------------------------

/// A contract whose WASM classifies `Fungible` gets a bespoke-Soroban
/// (`asset_type = 3`) row in `assets`, carrying only the surrogate
/// `contract_id` (empty code, issuer 0). Mirror of PG
/// `insert_assets_from_reclassified_contracts`.
#[test]
fn prepare_emits_soroban_asset_row_for_fungible_contract() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x9D);
    let contract = "C".to_string() + &"G".repeat(55);
    let wasm_hex = "33".repeat(32);
    let iface = fungible_classified_interface(&wasm_hex);
    let dep = ExtractedContractDeployment {
        contract_id: contract.clone(),
        wasm_hash: Some(wasm_hex.clone()),
        deployer_account: None,
        deployed_at_ledger: 10,
        contract_type: ContractType::Other,
        is_sac: false,
        sac_asset: None,
    };

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        std::slice::from_ref(&iface),
        std::slice::from_ref(&dep),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    let crow = &staged.contract_rows[0];
    assert_eq!(crow.contract_type, Some(ContractType::Fungible as i16));

    let asset = staged
        .asset_rows
        .iter()
        .find(|a| a.contract_id == crow.id && a.asset_type == domain::AssetFamily::Soroban as i16)
        .expect("fungible contract gets a Soroban (type-3) asset row");
    assert_eq!(asset.asset_code, "");
    assert_eq!(asset.issuer_id, 0);
}

/// An `Nft`-classified contract is NOT an asset — it must never produce a
/// Soroban asset row (it routes to `nfts` instead).
#[test]
fn prepare_no_soroban_asset_row_for_nft_contract() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x9E);
    let contract = "C".to_string() + &"H".repeat(55);
    let wasm_hex = "44".repeat(32);
    let iface = nft_classified_interface(&wasm_hex);
    let dep = ExtractedContractDeployment {
        contract_id: contract.clone(),
        wasm_hash: Some(wasm_hex.clone()),
        deployer_account: None,
        deployed_at_ledger: 10,
        contract_type: ContractType::Other,
        is_sac: false,
        sac_asset: None,
    };

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        std::slice::from_ref(&iface),
        std::slice::from_ref(&dep),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    let crow = &staged.contract_rows[0];
    assert_eq!(crow.contract_type, Some(ContractType::Nft as i16));
    let has_soroban = staged
        .asset_rows
        .iter()
        .any(|a| a.contract_id == crow.id && a.asset_type == domain::AssetFamily::Soroban as i16);
    assert!(
        !has_soroban,
        "Nft contract must NOT produce a Soroban asset row"
    );
}

// ---------------------------------------------------------------------------
// Task 0283 live G9 — event routing via cross-ledger `prior_contract_verdicts`
// ---------------------------------------------------------------------------

/// A transfer event arrives for a contract deployed in an EARLIER ledger (no
/// deploy here, so `verdict_by_contract` is empty for it). The writer supplied
/// the contract's `Nft` verdict via `prior_contract_verdicts` (read from
/// `soroban_contracts`), so `route_for` sends the row to the HOT bucket instead
/// of quarantine.
#[test]
fn prepare_routes_event_to_hot_via_prior_contract_verdict() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0xA5);
    let contract = "C".to_string() + &"K".repeat(55);
    let nft = synthetic_nft(&contract, "tk1");
    let ev = synthetic_nft_event(&tx.hash, &contract, "tk1", 0);
    let prior: std::collections::HashMap<String, ContractType> =
        std::collections::HashMap::from([(contract.clone(), ContractType::Nft)]);

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[], // no deploy this ledger — contract deployed earlier
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: std::slice::from_ref(&nft),
        nft_events: std::slice::from_ref(&ev),
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &prior,
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare_with_sac_overrides");

    assert_eq!(staged.nft_rows.len(), 1, "routed to hot via prior verdict");
    assert_eq!(staged.nft_pending_rows.len(), 0);
}

/// Same shape, but the prior verdict is `Token` (a SAC). The event must DROP,
/// not quarantine — closing the 0221 write-time SAC leak at routing time.
#[test]
fn prepare_drops_event_when_prior_contract_verdict_is_sac() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0xA6);
    let contract = "C".to_string() + &"L".repeat(55);
    let nft = synthetic_nft(&contract, "tk1");
    let ev = synthetic_nft_event(&tx.hash, &contract, "tk1", 0);
    let prior: std::collections::HashMap<String, ContractType> =
        std::collections::HashMap::from([(contract.clone(), ContractType::Token)]);

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: std::slice::from_ref(&nft),
        nft_events: std::slice::from_ref(&ev),
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &prior,
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare_with_sac_overrides");

    assert_eq!(staged.nft_rows.len(), 0, "SAC event dropped, not hot");
    assert_eq!(
        staged.nft_pending_rows.len(),
        0,
        "SAC event dropped, not pending"
    );
}

/// Baseline: no prior verdict and no deploy here → the event still routes to
/// quarantine (pre-G9 behaviour preserved when the map is empty / fail-open).
#[test]
fn prepare_routes_event_to_pending_without_prior_verdict() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0xA7);
    let contract = "C".to_string() + &"M".repeat(55);
    let nft = synthetic_nft(&contract, "tk1");
    let ev = synthetic_nft_event(&tx.hash, &contract, "tk1", 0);

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: std::slice::from_ref(&nft),
        nft_events: std::slice::from_ref(&ev),
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare_with_sac_overrides");

    assert_eq!(staged.nft_rows.len(), 0);
    assert_eq!(
        staged.nft_pending_rows.len(),
        1,
        "quarantined without a verdict"
    );
}

/// A SAC deploy is never reclassified from WASM — `is_sac` short-circuits the
/// override. Even if a (spurious) verdict is present in `prior_wasm_verdicts`
/// for the same hash, the SAC row stays `Token`.
#[test]
fn prepare_prior_wasm_verdict_leaves_sac_untouched() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x9B);
    let contract = "C".to_string() + &"E".repeat(55);
    let wasm_hex = "11".repeat(32);

    let dep = ExtractedContractDeployment {
        contract_id: contract.clone(),
        wasm_hash: Some(wasm_hex.clone()),
        deployer_account: None,
        deployed_at_ledger: 10,
        contract_type: ContractType::Token,
        is_sac: true,
        sac_asset: None,
    };
    let prior: std::collections::HashMap<[u8; 32], ContractType> =
        std::collections::HashMap::from([([0x11u8; 32], ContractType::Nft)]);

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: std::slice::from_ref(&dep),
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &prior,
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare_with_sac_overrides");

    let row = staged
        .contract_rows
        .iter()
        .find(|r| r.contract_id == contract)
        .expect("contract row present");
    assert_eq!(row.contract_type, Some(ContractType::Token as i16));
}

/// No verdict for the deploy's hash (empty map, no same-ledger interface) →
/// the contract keeps the parser default `Other`. This is the fail-open path:
/// the writer's prefetch found nothing, so behaviour is identical to pre-0283.
#[test]
fn prepare_keeps_other_when_no_prior_verdict() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x9C);
    let contract = "C".to_string() + &"F".repeat(55);
    let wasm_hex = "11".repeat(32);

    let dep = ExtractedContractDeployment {
        contract_id: contract.clone(),
        wasm_hash: Some(wasm_hex.clone()),
        deployer_account: None,
        deployed_at_ledger: 10,
        contract_type: ContractType::Other,
        is_sac: false,
        sac_asset: None,
    };

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: std::slice::from_ref(&dep),
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare_with_sac_overrides");

    let row = staged
        .contract_rows
        .iter()
        .find(|r| r.contract_id == contract)
        .expect("contract row present");
    assert_eq!(row.contract_type, Some(ContractType::Other as i16));
}

/// NFT row whose contract was deployed in the same ledger with a
/// definitive `Fungible` verdict is dropped entirely (zero rows in
/// either bucket). Locks in the filter-time drop semantics.
#[test]
fn prepare_drops_nft_row_when_contract_classified_fungible() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x91);
    let contract = "C".to_string() + &"B".repeat(55);
    let wasm_hex = "22".repeat(32);

    let iface = fungible_classified_interface(&wasm_hex);
    let dep = ExtractedContractDeployment {
        contract_id: contract.clone(),
        wasm_hash: Some(wasm_hex.clone()),
        deployer_account: None,
        deployed_at_ledger: 10,
        contract_type: ContractType::Other,
        is_sac: false,
        sac_asset: None,
    };
    let nft = synthetic_nft(&contract, "tk1");
    let ev = synthetic_nft_event(&tx.hash, &contract, "tk1", 0);

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        std::slice::from_ref(&iface),
        std::slice::from_ref(&dep),
        &[],
        &[],
        &[],
        &[],
        std::slice::from_ref(&nft),
        std::slice::from_ref(&ev),
        &[],
    )
    .expect("prepare");

    assert!(
        staged.nft_rows.is_empty(),
        "Fungible verdict: NFT row must drop, not route to hot"
    );
    assert!(
        staged.nft_pending_rows.is_empty(),
        "Fungible verdict: NFT row must drop, not route to pending"
    );
    assert!(staged.nft_ownership_rows.is_empty());
    assert!(staged.nft_ownership_pending_rows.is_empty());
}

/// NFT row whose contract is NOT deployed in the same ledger (no
/// classifier verdict accessible at stage time) routes to the
/// quarantine `nft_pending_rows` bucket. Mirrors PG `Other`/uncached
/// semantics — CH has no DB access in stage, so prior-ledger
/// classifications are unreachable here.
#[test]
fn prepare_routes_unclassified_contract_nft_to_pending_bucket() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x92);
    let contract = "C".to_string() + &"C".repeat(55);
    let nft = synthetic_nft(&contract, "tk1");
    let ev = synthetic_nft_event(&tx.hash, &contract, "tk1", 0);

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        std::slice::from_ref(&nft),
        std::slice::from_ref(&ev),
        &[],
    )
    .expect("prepare");

    assert!(
        staged.nft_rows.is_empty(),
        "Unclassified contract: nothing in hot"
    );
    assert_eq!(
        staged.nft_pending_rows.len(),
        1,
        "Unclassified contract: row in pending bucket"
    );
    assert_eq!(staged.nft_ownership_rows.len(), 0);
    assert_eq!(staged.nft_ownership_pending_rows.len(), 1);
}

// ---------------------------------------------------------------------------
// Task 0323 — un-deployed SAC modelled as asset (was task 0220 re-insert)
// ---------------------------------------------------------------------------

/// Task 0323 → ADR 0051 — an un-deployed SAC (in `sac_overrides`, not deployed
/// this ledger) is modelled as an ASSET FACET, not a contract: NO
/// `soroban_contracts` row is written for it (the Pass-2 FK stub is suppressed),
/// and the SAC handle is folded onto the underlying classic/native asset row
/// (`sac_contract_id` set, `sac_deployed = false`). (Was the task-0220 skeleton.)
#[test]
fn prepare_models_undeployed_sac_override_as_asset_not_contract() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0xA0);

    // XLM SAC contract_id (well-known mainnet address). Production feeds these
    // ready-made from `detect_undeployed_sac_overrides` over the ledger's events.
    let xlm_sac = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
    let overrides = vec![SacOverride {
        contract_id: xlm_sac.to_string(),
        identity: SacAssetIdentity::Native,
    }];

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &overrides,
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare_with_sac_overrides");

    // No soroban_contracts row for the un-deployed SAC — it is an asset now.
    assert!(
        !staged
            .contract_rows
            .iter()
            .any(|r| r.contract_id == xlm_sac),
        "un-deployed SAC override writes NO contract row",
    );
    // The native (type=0) identity row exists (one, merged with the singleton) —
    // and the SAC handle is folded into `asset_sac`, NOT onto the `assets` row
    // (ADR 0051 — the assets table is a no-version RMT, so the facet lives in the
    // AggregatingMergeTree side table). `sac_deployed = 0` for an un-deployed override.
    let native_rows: Vec<&_> = staged
        .asset_rows
        .iter()
        .filter(|a| a.asset_type == domain::AssetFamily::Native as i16)
        .collect();
    assert_eq!(
        native_rows.len(),
        1,
        "one native asset identity row (override merged)"
    );
    assert_eq!(native_rows[0].asset_code, "");
    assert_eq!(native_rows[0].issuer_id, 0);

    let sac_rows: Vec<&_> = staged
        .asset_sac_rows
        .iter()
        .filter(|s| s.asset_type == domain::AssetFamily::Native as i16)
        .collect();
    assert_eq!(
        sac_rows.len(),
        1,
        "one asset_sac facet row for the native SAC"
    );
    assert_eq!(sac_rows[0].asset_code, "");
    assert_eq!(sac_rows[0].issuer_id, 0);
    assert_eq!(
        sac_rows[0].contract_id, 0,
        "facet keyed on the carrier (contract_id 0)"
    );
    assert_eq!(
        sac_rows[0].sac_contract_id,
        super::ids::contract_id(xlm_sac),
        "SAC handle stored as the C… surrogate",
    );
    assert_eq!(
        sac_rows[0].sac_deployed, 0,
        "un-deployed override → sac_deployed = 0",
    );
    // No retired `asset_type = 2` rows — that value is gone (ADR 0051).
    assert!(
        !staged.asset_rows.iter().any(|a| a.asset_type == 2),
        "no retired type=2 rows emitted",
    );
}

/// When the same contract is deployed in the current ledger (real
/// deploy with `is_sac=true` from parser metadata) AND surfaces in
/// `sac_overrides`, the deploy-time row wins and the override is
/// suppressed (no duplicate emission). Locks in the dedup precedence.
#[test]
fn prepare_skips_sac_override_when_contract_deployed_same_ledger() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0xA1);

    let xlm_sac = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
    let dep = ExtractedContractDeployment {
        contract_id: xlm_sac.to_string(),
        wasm_hash: None,
        deployer_account: None,
        deployed_at_ledger: 10,
        contract_type: ContractType::Token,
        is_sac: true,
        sac_asset: Some(SacAssetIdentity::Native),
    };
    let overrides = vec![SacOverride {
        contract_id: xlm_sac.to_string(),
        identity: SacAssetIdentity::Native,
    }];

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: std::slice::from_ref(&dep),
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &overrides,
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare_with_sac_overrides");

    let sac_rows: Vec<&_> = staged
        .contract_rows
        .iter()
        .filter(|r| r.contract_id == xlm_sac)
        .collect();
    assert_eq!(sac_rows.len(), 1, "deploy-time row preserved, no duplicate");
    let row = sac_rows[0];
    assert!(row.is_sac);
    assert_eq!(
        row.wasm_uploaded_at_ledger, 10,
        "real deploy carries the deploy ledger as version"
    );

    // ADR 0051: one asset_sac facet row, `sac_deployed = 1` — the deploy sighting
    // (1) `max`-beats the same-ledger override sighting (0) via push_sac.
    let facet: Vec<&_> = staged
        .asset_sac_rows
        .iter()
        .filter(|s| s.asset_type == domain::AssetFamily::Native as i16)
        .collect();
    assert_eq!(facet.len(), 1, "one native asset_sac facet row");
    assert_eq!(
        facet[0].sac_contract_id,
        super::ids::contract_id(xlm_sac),
        "facet carries the SAC surrogate",
    );
    assert_eq!(
        facet[0].sac_deployed, 1,
        "deploy sighting wins over override (max-merge)",
    );
}

/// ADR 0051 regression — the clobber bug the columns-on-`assets` design had. A
/// ledger with ONLY trustline activity (a classic_credit asset, no SAC sighting)
/// emits its identity row but ZERO `asset_sac` rows, so a per-ledger re-emit can
/// never zero a previously-recorded SAC facet (the facet lives in the
/// AggregatingMergeTree side table, written only on a SAC sighting).
#[test]
fn prepare_trustline_only_ledger_emits_no_sac_facet() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0xA2);
    let usdc = ExtractedAsset {
        asset_type: domain::AssetFamily::ClassicCredit,
        asset_code: Some("USDC".to_string()),
        issuer_address: Some(
            "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN".to_string(),
        ),
        contract_id: None,
        sac_contract_id: None,
        sac_deployed: false,
    };

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: std::slice::from_ref(&usdc),
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare_with_sac_overrides");

    // Identity row present…
    assert!(
        staged
            .asset_rows
            .iter()
            .any(|a| a.asset_type == 1 && a.asset_code == "USDC"),
        "classic_credit identity row emitted",
    );
    // …but NO facet row → nothing that could clobber a prior SAC facet.
    assert!(
        staged.asset_sac_rows.is_empty(),
        "trustline-only ledger writes no asset_sac rows",
    );
}

// ---- Task 0320: live WASM-upgrade row (build_wasm_upgrade_rows) ----

fn executable_update_event(contract: &str) -> ExtractedEvent {
    // topics = [Symbol("executable_update"), vec[Symbol("Wasm"), Bytes(old=0x11)],
    //                                        vec[Symbol("Wasm"), Bytes(new=0x22)]]
    ExtractedEvent {
        transaction_hash: "abcd".into(),
        event_type: ContractEventType::System,
        source: EventSource::TxLevel,
        contract_id: Some(contract.to_string()),
        topics: serde_json::json!([
            {"type":"sym","value":"executable_update"},
            {"type":"vec","value":[{"type":"sym","value":"Wasm"},{"type":"bytes","value":"ERERERERERERERERERERERERERERERERERERERERERE="}]},
            {"type":"vec","value":[{"type":"sym","value":"Wasm"},{"type":"bytes","value":"IiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiI="}]}
        ]),
        data: serde_json::json!({"type":"vec","value":[]}),
        event_index: 0,
        op_index: None,
        stage: None,
        ledger_sequence: 555,
        created_at: 1_700_000_000,
    }
}

/// A prior `soroban_contracts` row for the upgrade-prefetch map. Only the
/// identity columns are meaningful here; `wasm_hash` / `wasm_uploaded_at_ledger`
/// are the pre-upgrade values that `build_wasm_upgrade_rows` overrides.
fn prior_contract_row(
    addr: &str,
    deployer_id: Option<i64>,
    deployed_at_ledger: Option<i64>,
    contract_type: Option<i16>,
    is_sac: bool,
) -> SorobanContractRow {
    SorobanContractRow {
        id: ids::contract_id(addr),
        contract_id: addr.to_string(),
        wasm_hash: Some([0x11u8; 32]),
        wasm_uploaded_at_ledger: deployed_at_ledger.unwrap_or(0),
        deployer_id,
        deployed_at_ledger,
        contract_type,
        is_sac,
    }
}

#[test]
fn build_wasm_upgrade_rows_carries_identity_and_overrides_hash() {
    let addr = "C".to_string() + &"U".repeat(55);
    let events = vec![("abcd".to_string(), vec![executable_update_event(&addr)])];

    let mut prior = std::collections::HashMap::new();
    prior.insert(
        addr.clone(),
        prior_contract_row(&addr, Some(7), Some(100), Some(1), false),
    );

    let rows = stage::build_wasm_upgrade_rows(&events, &prior, 555);
    assert_eq!(rows.len(), 1, "one upgrade row");
    let r = &rows[0];
    assert_eq!(r.contract_id, addr);
    assert_eq!(r.id, ids::contract_id(&addr));
    assert_eq!(r.wasm_hash, Some([0x22u8; 32]), "overridden to NEW hash");
    assert_eq!(
        r.wasm_uploaded_at_ledger, 555,
        "RMT version = upgrade ledger"
    );
    assert_eq!(r.deployer_id, Some(7), "deployer carried forward");
    assert_eq!(r.deployed_at_ledger, Some(100), "deploy ledger carried");
    assert_eq!(r.contract_type, Some(1), "verdict carried (no flip)");
    assert!(!r.is_sac);
}

#[test]
fn build_wasm_upgrade_rows_skips_when_no_prior_row() {
    // No prior row → must NOT emit: a partial row would clobber identity
    // columns to NULL under RMT whole-row replace.
    let addr = "C".to_string() + &"V".repeat(55);
    let events = vec![("abcd".to_string(), vec![executable_update_event(&addr)])];
    let prior = std::collections::HashMap::new();
    assert!(stage::build_wasm_upgrade_rows(&events, &prior, 555).is_empty());
}

#[test]
fn build_wasm_upgrade_rows_ignores_non_upgrade_events() {
    let addr = "C".to_string() + &"W".repeat(55);
    let mut ev = executable_update_event(&addr);
    ev.topics = serde_json::json!([{"type":"sym","value":"transfer"}]);
    let events = vec![("abcd".to_string(), vec![ev])];
    let mut prior = std::collections::HashMap::new();
    prior.insert(
        addr.clone(),
        prior_contract_row(&addr, Some(7), Some(100), Some(1), false),
    );
    assert!(stage::build_wasm_upgrade_rows(&events, &prior, 555).is_empty());
}

#[test]
fn build_wasm_upgrade_rows_ignores_diagnostic_source() {
    // A diagnostic-container copy (or a failed-tx event) must NOT drive a write —
    // it can carry a hash the chain never applied. Mirrors the soroban_events
    // staging guard + the backfill's already-filtered source table.
    let addr = "C".to_string() + &"X".repeat(55);
    let mut ev = executable_update_event(&addr);
    ev.source = EventSource::Diagnostic;
    let events = vec![("abcd".to_string(), vec![ev])];
    let mut prior = std::collections::HashMap::new();
    prior.insert(
        addr.clone(),
        prior_contract_row(&addr, Some(7), Some(100), Some(1), false),
    );
    assert!(stage::build_wasm_upgrade_rows(&events, &prior, 555).is_empty());
}

#[test]
fn build_wasm_upgrade_rows_ignores_non_system_event_type() {
    // A Contract-typed event with executable_update-shaped topics is a spoof —
    // only host-emitted System events may rewrite wasm_hash.
    let addr = "C".to_string() + &"Z".repeat(55);
    let mut ev = executable_update_event(&addr);
    ev.event_type = ContractEventType::Contract;
    let events = vec![("abcd".to_string(), vec![ev])];
    let mut prior = std::collections::HashMap::new();
    prior.insert(
        addr.clone(),
        prior_contract_row(&addr, Some(7), Some(100), Some(1), false),
    );
    assert!(stage::build_wasm_upgrade_rows(&events, &prior, 555).is_empty());
}

#[test]
fn build_wasm_upgrade_rows_carries_is_sac_from_prior() {
    // is_sac rides along from the read-back row (matches the backfill SQL, which
    // also passes it through). No upgrader is a mislabeled SAC on current data,
    // so carry-forward and force-false are equivalent in practice.
    let addr = "C".to_string() + &"Y".repeat(55);
    let events = vec![("abcd".to_string(), vec![executable_update_event(&addr)])];
    let mut prior = std::collections::HashMap::new();
    prior.insert(
        addr.clone(),
        prior_contract_row(&addr, None, None, Some(1), true),
    );
    let rows = stage::build_wasm_upgrade_rows(&events, &prior, 555);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_sac, "is_sac carried forward from the prior row");
}

/// ADR 0055 — the write path must stamp `closed_at_ledger` on exactly the rows
/// whose ledger entry disappeared, and on no others. Before this column a
/// removal was written as `amount = 0`, byte-identical to a live-but-empty
/// holding, so the read path could only hide both (issue #377).
#[test]
fn closed_at_ledger_marks_only_real_closures() {
    let ledger = synthetic_ledger();

    // One live account: zero XLM (legal — sponsored reserves, CAP-0033), one
    // live trustline sitting at zero, and one trustline that was REMOVED.
    let live = ExtractedAccountState {
        account_id: "GLIVE".to_string(),
        first_seen_ledger: None,
        last_seen_ledger: 100,
        sequence_number: 7,
        balances: serde_json::json!([
            {"asset_type": "native", "balance": "0.0000000"},
            {"asset_type": "credit_alphanum4", "asset_code": "AQUA",
             "issuer": "GISSUER", "balance": "0.0000000"},
        ]),
        removed_trustlines: vec![serde_json::json!({
            "asset_type": "credit_alphanum4", "asset_code": "SHX", "issuer": "GISSUER",
        })],
        account_removed: false,
        signers: Some(vec![
            serde_json::json!({"key": "GSIGNER1", "weight": 1, "type": "ed25519"}),
            serde_json::json!({"key": "TSIGNER2", "weight": 255, "type": "preauth_tx"}),
        ]),
        thresholds: Some("01030303".to_string()),
        flags: Some(0),
        home_domain: None,
        created_at: 1_700_000_000,
    };

    // A merged account: the native 0 is a tombstone, not a balance.
    let merged = ExtractedAccountState {
        account_id: "GMERGED".to_string(),
        first_seen_ledger: None,
        last_seen_ledger: 100,
        sequence_number: -1,
        balances: serde_json::json!([{"asset_type": "native", "balance": "0.0000000"}]),
        removed_trustlines: vec![],
        account_removed: true,
        signers: None,
        thresholds: None,
        flags: None,
        home_domain: None,
        created_at: 1_700_000_000,
    };

    let staged = stage::prepare(
        &ledger,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[live, merged],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    let live_id = ids::account_id("GLIVE");
    let merged_id = ids::account_id("GMERGED");
    let find = |holder: i64, asset: i64| {
        staged
            .unified_balance_rows
            .iter()
            .find(|r| r.holder_id == holder && r.asset_id == asset)
            .unwrap_or_else(|| panic!("no balance row for holder {holder} asset {asset}"))
    };

    // Live account, zero XLM — the row the page must start showing.
    assert_eq!(find(live_id, ids::NATIVE_ASSET_ID).closed_at_ledger, 0);
    // Live trustline at zero — same amount as a closure, different meaning.
    assert_eq!(
        find(live_id, ids::credit_asset_id("AQUA", "GISSUER")).closed_at_ledger,
        0
    );
    // Removed trustline — stamped with the ledger it disappeared in.
    assert_eq!(
        find(live_id, ids::credit_asset_id("SHX", "GISSUER")).closed_at_ledger,
        100
    );
    // Merged account — the native tombstone is a closure, not a zero balance.
    let tombstone = find(merged_id, ids::NATIVE_ASSET_ID);
    assert_eq!(tombstone.amount, 0);
    assert_eq!(
        tombstone.closed_at_ledger, 100,
        "an account_merge tombstone must be marked closed, or merged accounts \
         render as holding 0 XLM once the read filter flips"
    );
}

/// lore-0463: the signers side row follows full-set-replace semantics and is
/// emitted ONLY when the AccountEntry itself was observed.
#[test]
fn entry_state_rows_full_set_replace_semantics() {
    let ledger = synthetic_ledger();

    // Entry observed, thresholds 01030303 (master 1, low/med/high 3), two
    // signers — the issue #377 fixture shape.
    let observed = ExtractedAccountState {
        account_id: "GOBSERVED".to_string(),
        first_seen_ledger: None,
        last_seen_ledger: 100,
        sequence_number: 7,
        home_domain: None,
        created_at: 1_700_000_000,
        balances: serde_json::json!([]),
        removed_trustlines: vec![],
        account_removed: false,
        signers: Some(vec![
            serde_json::json!({"key": "GS1", "weight": 1, "type": "ed25519"}),
            serde_json::json!({"key": "XS2", "weight": 3, "type": "hash_x"}),
        ]),
        thresholds: Some("01030303".to_string()),
        flags: Some(5),
    };
    // Entry observed with an EMPTY set — removing the last signer must still
    // emit a row, or the stale set survives in the RMT forever.
    let emptied = ExtractedAccountState {
        account_id: "GEMPTIED".to_string(),
        first_seen_ledger: None,
        last_seen_ledger: 100,
        sequence_number: 9,
        home_domain: None,
        created_at: 1_700_000_000,
        balances: serde_json::json!([]),
        removed_trustlines: vec![],
        account_removed: false,
        signers: Some(vec![]),
        thresholds: Some("01000000".to_string()),
        flags: Some(0),
    };
    // Trustline-only accum: NO entry observed — must not touch the set.
    let trustline_only = ExtractedAccountState {
        account_id: "GTRUSTONLY".to_string(),
        first_seen_ledger: None,
        last_seen_ledger: 100,
        sequence_number: -1,
        home_domain: None,
        created_at: 1_700_000_000,
        balances: serde_json::json!([
            {"asset_type": "credit_alphanum4", "asset_code": "AQUA",
             "issuer": "GISSUER", "balance": "1.0000000"},
        ]),
        removed_trustlines: vec![],
        account_removed: false,
        signers: None,
        thresholds: None,
        flags: None,
    };
    // Merged account: nothing to emit.
    let merged = ExtractedAccountState {
        account_id: "GMERGED2".to_string(),
        first_seen_ledger: None,
        last_seen_ledger: 100,
        sequence_number: -1,
        home_domain: None,
        created_at: 1_700_000_000,
        balances: serde_json::json!([{"asset_type": "native", "balance": "0.0000000"}]),
        removed_trustlines: vec![],
        account_removed: true,
        signers: None,
        thresholds: None,
        flags: None,
    };

    let staged = stage::prepare(
        &ledger,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[observed, emptied, trustline_only, merged],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(
        staged.account_entry_state_rows.len(),
        2,
        "exactly the two entry-observed accounts emit signer rows"
    );
    let obs = staged
        .account_entry_state_rows
        .iter()
        .find(|r| r.account_id == ids::account_id("GOBSERVED"))
        .expect("observed row");
    assert_eq!(obs.master_weight, 1);
    assert_eq!(
        (obs.threshold_low, obs.threshold_med, obs.threshold_high),
        (3, 3, 3),
        "thresholds hex must parse as [master, low, med, high] — a byte-order \
         swap here mislabels every multisig account"
    );
    assert_eq!(obs.signer_keys, vec!["GS1", "XS2"]);
    assert_eq!(obs.signer_weights, vec![1, 3]);
    assert_eq!(obs.signer_types, vec!["ed25519", "hash_x"]);
    assert_eq!(obs.flags, 5);
    assert_eq!(obs.last_updated_ledger, 100);

    let emp = staged
        .account_entry_state_rows
        .iter()
        .find(|r| r.account_id == ids::account_id("GEMPTIED"))
        .expect("emptied row");
    assert!(
        emp.signer_keys.is_empty(),
        "an emptied set must write an empty row, not skip the write"
    );
    assert_eq!(emp.master_weight, 1);
}

/// The Soroban (type-3) closure stamp. A holder who spent down to zero and a
/// holder whose `ContractData` entry was REMOVED both carry `balance = 0`, so
/// `closed` is the only thing separating them — the same ambiguity as a classic
/// trustline, in a different write path (ADR 0055). In scope for this task and
/// previously asserted nowhere.
#[test]
fn soroban_removal_stamps_closed_at_ledger_but_a_spent_down_holder_does_not() {
    use xdr_parser::ExtractedSorobanBalance;

    let spent = ExtractedSorobanBalance {
        contract_id: "CTOKEN".to_string(),
        holder: "GSPENT".to_string(),
        balance: 0,
        ledger: 100,
        closed: false,
    };
    let removed = ExtractedSorobanBalance {
        holder: "GREMOVED".to_string(),
        closed: true,
        ..spent.clone()
    };

    let rows = stage::build_balance_rows(&[spent, removed], &HashMap::new());
    let by = |strkey: &str| {
        rows.iter()
            .find(|r| r.holder_id == ids::address_id(strkey))
            .expect("row")
    };

    assert_eq!(by("GSPENT").amount, 0);
    assert_eq!(
        by("GSPENT").closed_at_ledger,
        0,
        "a live holder at zero must stay live — this is the whole bug"
    );
    assert_eq!(by("GREMOVED").amount, 0);
    assert_eq!(
        by("GREMOVED").closed_at_ledger,
        100,
        "a removed entry must carry the ledger it disappeared in"
    );
}

/// The in-ledger last-wins fold, for the four state writers that lacked a
/// regression test (full-schema audit, task 0503): two states for one key in
/// ONE ledger must collapse to a single row carrying the LAST state in
/// application order. Two rows would tie the RMT version and the merge would
/// pick arbitrarily — the `balances` defect, in any other table.
#[test]
fn same_ledger_state_pairs_collapse_to_the_last_for_every_state_writer() {
    let ledger = synthetic_ledger();
    let seq = i64::from(ledger.sequence);

    // -- accounts: sequence bump then home_domain set, same ledger --
    let first = ExtractedAccountState {
        account_id: "GFOLD".to_string(),
        first_seen_ledger: None,
        last_seen_ledger: ledger.sequence,
        sequence_number: 7,
        home_domain: None,
        created_at: 1_700_000_000,
        balances: serde_json::json!([]),
        removed_trustlines: vec![],
        account_removed: false,
        signers: None,
        thresholds: None,
        flags: None,
    };
    let second = ExtractedAccountState {
        sequence_number: 9,
        home_domain: Some("example.org".to_string()),
        ..first.clone()
    };

    // -- lp_positions: deposit then partial withdraw, same ledger --
    let pool_hex = "ab".repeat(32);
    let deposit = ExtractedLpPosition {
        pool_id: pool_hex.clone(),
        account_id: "GFOLD".to_string(),
        shares: "2.0000000".to_string(),
        first_deposit_ledger: Some(ledger.sequence),
        last_updated_ledger: ledger.sequence,
        closed: false,
    };
    let withdraw = ExtractedLpPosition {
        shares: "1.0000000".to_string(),
        first_deposit_ledger: None,
        ..deposit.clone()
    };

    // -- liquidity_pools: created then touched again, same ledger --
    let pool = ExtractedLiquidityPool {
        pool_id: pool_hex.clone(),
        asset_a: serde_json::json!("native"),
        asset_b: serde_json::json!({"type": "credit_alphanum4", "code": "USDC", "issuer": "GISS"}),
        fee_bps: 30,
        reserves: serde_json::json!({}),
        total_shares: "2.0000000".to_string(),
        created_at_ledger: Some(ledger.sequence),
        last_updated_ledger: ledger.sequence,
        created_at: 1_700_000_000,
    };
    let pool_again = ExtractedLiquidityPool {
        created_at_ledger: None,
        total_shares: "1.0000000".to_string(),
        ..pool.clone()
    };

    let staged = stage::prepare(
        &ledger,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[first, second],
        &[pool, pool_again],
        &[],
        &[],
        &[],
        &[],
        &[deposit, withdraw],
    )
    .expect("prepare");

    let acct = staged
        .account_rows
        .iter()
        .find(|r| r.account_id == "GFOLD")
        .expect("one account row");
    assert_eq!(
        staged
            .account_rows
            .iter()
            .filter(|r| r.account_id == "GFOLD")
            .count(),
        1,
        "two same-ledger states must not emit two rows at one RMT version"
    );
    assert_eq!(acct.sequence_number, 9, "last state in tx order wins");
    assert_eq!(acct.home_domain.as_deref(), Some("example.org"));

    assert_eq!(staged.lp_position_rows.len(), 1);
    assert_eq!(
        staged.lp_position_rows[0].shares, 10_000_000,
        "the withdraw (last in order) is the surviving share count"
    );
    assert_eq!(
        staged.lp_position_rows[0].first_deposit_ledger, seq,
        "first_deposit survives the overwrite via min-preservation"
    );

    assert_eq!(
        staged.pool_rows.len(),
        1,
        "one pool row per ledger, not one per touch"
    );
    // Legs-migration step 2: a CLASSIC row fills `legs` too — ASSET
    // surrogates (the lp_operation_amounts join key), derived from the same
    // pair the legacy columns carry, so the pair can eventually retire.
    let pr = &staged.pool_rows[0];
    assert_eq!(pr.pool_kind, 0);
    assert_eq!(
        pr.legs,
        vec![
            ids::pool_leg_asset_id(pr.asset_a_type, &pr.asset_a_code, pr.asset_a_issuer_id),
            ids::pool_leg_asset_id(pr.asset_b_type, &pr.asset_b_code, pr.asset_b_issuer_id),
        ],
        "classic legs are the pair's asset surrogates, in order"
    );
}

/// Two ownership events for one NFT in ONE ledger: the later event's owner is
/// the row that survives — mirrors the `>=` in the hot-bucket fold.
#[test]
fn same_ledger_nft_owner_flip_keeps_the_last_owner() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x91);
    let contract = "C".to_string() + &"B".repeat(55);
    let wasm_hex = "22".repeat(32);
    let iface = nft_classified_interface(&wasm_hex);
    let dep = ExtractedContractDeployment {
        contract_id: contract.clone(),
        wasm_hash: Some(wasm_hex.clone()),
        deployer_account: None,
        deployed_at_ledger: 10,
        contract_type: ContractType::Other,
        is_sac: false,
        sac_asset: None,
    };
    let minted = ExtractedNft {
        owner_account: Some("GFIRST".to_string()),
        ..synthetic_nft(&contract, "tk1")
    };
    let transferred = ExtractedNft {
        owner_account: Some("GSECOND".to_string()),
        ..minted.clone()
    };
    let ev = synthetic_nft_event(&tx.hash, &contract, "tk1", 0);

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &[],
        &[],
        std::slice::from_ref(&iface),
        std::slice::from_ref(&dep),
        &[],
        &[],
        &[],
        &[],
        &[minted, transferred],
        std::slice::from_ref(&ev),
        &[],
    )
    .expect("prepare");

    assert_eq!(staged.nft_rows.len(), 1, "one row per token per ledger");
    assert_eq!(
        staged.nft_rows[0].current_owner_id,
        Some(ids::account_id("GSECOND")),
        "the LAST transfer in application order owns the token at ledger end"
    );
}

/// Two transactions in ONE ledger touching the same account must collapse to a
/// single `account_entry_state` row carrying the LAST state.
///
/// `extract_account_states` runs per transaction and every state in a ledger
/// carries that ledger as its watermark, so two rows would share the RMT
/// version — and `ReplacingMergeTree` resolves a tie arbitrarily. The loser
/// could be the newer one, leaving a REMOVED signer as the surviving row: the
/// exact ghost the whole-set-replacement design promises is impossible.
#[test]
fn two_states_for_one_account_in_one_ledger_collapse_to_the_last() {
    let ledger = synthetic_ledger();
    // tx #1 — signer S is present.
    let added = ExtractedAccountState {
        account_id: "GTWICE".to_string(),
        first_seen_ledger: None,
        last_seen_ledger: 100,
        sequence_number: 7,
        home_domain: None,
        created_at: 1_700_000_000,
        balances: serde_json::json!([]),
        removed_trustlines: vec![],
        account_removed: false,
        signers: Some(vec![serde_json::json!({
            "key": "GS1", "weight": 1, "type": "ed25519"
        })]),
        thresholds: Some("01020202".to_string()),
        flags: Some(0),
    };
    // tx #5, SAME ledger — signer S removed. This is the state that must win.
    let removed = ExtractedAccountState {
        signers: Some(vec![]),
        ..added.clone()
    };

    let staged = stage::prepare(
        &ledger,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[added, removed],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert_eq!(
        staged.account_entry_state_rows.len(),
        1,
        "two same-ledger states must not emit two rows at the same RMT version — \
         the merge would pick a winner arbitrarily"
    );
    assert!(
        staged.account_entry_state_rows[0].signer_keys.is_empty(),
        "last state in ledger/tx order wins; a surviving 'GS1' means a removed \
         signer ghosted"
    );
}

#[test]
fn prepare_registers_a_pool_from_a_real_add_pool_event() {
    // Verbatim mainnet payload (router CBQDHNBF…6QUK) — the same fixture the
    // pool_router corpus test pins, so decoder and staging cannot drift apart.
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x61);
    let router = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK";
    let pool = "CDTSSTLKVVPWJZXVCGJJNGWKH5MY7OMINVXTB7DGFMDJTCCDBCSRG52O";

    let ev = ExtractedEvent {
        transaction_hash: tx.hash.clone(),
        event_type: ContractEventType::Contract,
        source: EventSource::TxLevel,
        contract_id: Some(router.to_string()),
        topics: serde_json::json!([
            {"type": "sym", "value": "add_pool"},
            {"type": "vec", "value": [
                {"type": "address", "value": "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"},
                {"type": "address", "value": "CDLWTKL7XIALOQPTV7R2KKTXTA6OPKT4T354Y7RG7S6TERQ7KI2VPXIW"}
            ]}
        ]),
        data: serde_json::json!({"type": "vec", "value": [
            {"type": "address", "value": pool},
            {"type": "sym", "value": "constant"},
            {"type": "bytes", "value": "suAvz8pslvitXL2E53hKd3s22clqJFlALE9FhGKqt/A="},
            {"type": "vec", "value": [{"type": "u32", "value": 10}]}
        ]}),
        event_index: 0,
        op_index: None,
        stage: None,
        ledger_sequence: 10,
        created_at: 1_700_000_000,
    };
    let events = vec![(tx.hash.clone(), vec![ev])];
    // The pool's own instance, written in the SAME transaction as `add_pool`
    // on mainnet (probed on raw meta) — it names this router, which is what
    // lets the registration through the corroboration guard.
    let instances = [pool_instance_declaring(pool, router)];

    let staged = stage_registration(&ledger, &tx, &events, &instances);

    // The event row itself still lands — registration is IN ADDITION, never
    // instead of the raw event.
    assert_eq!(staged.event_rows.len(), 1);

    let rows: Vec<_> = staged
        .pool_rows
        .iter()
        .filter(|r| r.pool_kind == 1)
        .collect();
    assert_eq!(rows.len(), 1, "one registration, one registry row");
    let row = rows[0];
    assert_eq!(row.pool_type_raw, "constant");
    assert_eq!(row.fee_bps, 10, "fee comes from init_args[0]");
    assert_eq!(row.legs.len(), 2, "legs are asset surrogates, in order");
    assert_eq!(row.deployment_id, ids::contract_id(router));
    // pool_id is the raw C-address payload, not a hash of anything.
    assert_eq!(
        stellar_strkey::Contract(row.pool_id)
            .to_string()
            .to_string(),
        pool
    );
    // Classic columns stay at their defaults on a soroban row.
    assert_eq!(row.asset_a_type, 0);
    assert!(row.asset_a_code.is_empty());
}

#[test]
fn prepare_ignores_non_registrations_and_labelled_topics() {
    // `trade` (another protocol's collision-prone name) and the Soroswap
    // labelled shape must produce NO registry rows.
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x62);
    let contract = "C".to_string() + &"F".repeat(55);

    let make = |topics: serde_json::Value| ExtractedEvent {
        transaction_hash: tx.hash.clone(),
        event_type: ContractEventType::Contract,
        source: EventSource::TxLevel,
        contract_id: Some(contract.clone()),
        topics,
        data: serde_json::json!({"type": "vec", "value": []}),
        event_index: 0,
        op_index: None,
        stage: None,
        ledger_sequence: 10,
        created_at: 1_700_000_000,
    };
    let events = vec![(
        tx.hash.clone(),
        vec![
            make(serde_json::json!([{"type": "sym", "value": "trade"}])),
            make(serde_json::json!([
                {"type": "string", "value": "SoroswapPair"},
                {"type": "sym", "value": "add_pool"}
            ])),
        ],
    )];

    let staged = stage::prepare(
        &ledger,
        std::slice::from_ref(&tx),
        &[(tx.hash.clone(), vec![])],
        &events,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .expect("prepare");

    assert!(
        staged.pool_rows.iter().all(|r| r.pool_kind == 0),
        "no soroban registry row may come from a non-registration"
    );
}

#[test]
fn prepare_refuses_a_registration_with_an_unparseable_fee() {
    // A shape where init_args[0] is not a number is a vocabulary nobody has
    // seen (497/497 mainnet registrations carry a u32 there). It must be
    // refused loudly, never recorded as a plausible fee of 0.
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x63);
    let router = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK";

    let ev = ExtractedEvent {
        transaction_hash: tx.hash.clone(),
        event_type: ContractEventType::Contract,
        source: EventSource::TxLevel,
        contract_id: Some(router.to_string()),
        topics: serde_json::json!([
            {"type": "sym", "value": "add_pool"},
            {"type": "vec", "value": [
                {"type": "address", "value": "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"},
                {"type": "address", "value": "CDLWTKL7XIALOQPTV7R2KKTXTA6OPKT4T354Y7RG7S6TERQ7KI2VPXIW"}
            ]}
        ]),
        data: serde_json::json!({"type": "vec", "value": [
            {"type": "address", "value": "CDTSSTLKVVPWJZXVCGJJNGWKH5MY7OMINVXTB7DGFMDJTCCDBCSRG52O"},
            {"type": "sym", "value": "constant"},
            {"type": "bytes", "value": "suAvz8pslvitXL2E53hKd3s22clqJFlALE9FhGKqt/A="},
            {"type": "vec", "value": [{"type": "sym", "value": "not_a_fee"}]}
        ]}),
        event_index: 0,
        op_index: None,
        stage: None,
        ledger_sequence: 10,
        created_at: 1_700_000_000,
    };
    let events = vec![(tx.hash.clone(), vec![ev])];

    // The pool DOES corroborate this router, so the registration reaches the
    // fee check — without this the refusal below would come from the
    // corroboration guard and this test would pass for the wrong reason.
    let instance = xdr_parser::pool_state::ExtractedPoolInstance {
        state: xdr_parser::pool_state::PoolInstanceState {
            pool: "CDTSSTLKVVPWJZXVCGJJNGWKH5MY7OMINVXTB7DGFMDJTCCDBCSRG52O".into(),
            token_share: None,
            total_shares: None,
            plane: Some("CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY".into()),
            router: Some(router.to_string()),
            reserves: Vec::new(),
        },
        ledger_sequence: 10,
        created: true,
    };

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &events,
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[xdr_parser::pool_family::PoolFamilyWrite::RouterPool(
            instance.clone(),
        )],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare itself succeeds — one refused registration must not fail the ledger");

    assert!(
        staged.pool_rows.iter().all(|r| r.pool_kind == 0),
        "no registry row may carry a fabricated fee"
    );
    assert_eq!(staged.event_rows.len(), 1, "the raw event still lands");
}

/// Build an `add_pool` event for `pool`, emitted by `router`, from the
/// mainnet-verbatim payload shape.
#[cfg(test)]
fn add_pool_event(tx_hash: &str, router: &str, pool: &str, source: EventSource) -> ExtractedEvent {
    ExtractedEvent {
        transaction_hash: tx_hash.to_string(),
        event_type: ContractEventType::Contract,
        source,
        contract_id: Some(router.to_string()),
        topics: serde_json::json!([
            {"type": "sym", "value": "add_pool"},
            {"type": "vec", "value": [
                {"type": "address", "value": "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"},
                {"type": "address", "value": "CDLWTKL7XIALOQPTV7R2KKTXTA6OPKT4T354Y7RG7S6TERQ7KI2VPXIW"}
            ]}
        ]),
        data: serde_json::json!({"type": "vec", "value": [
            {"type": "address", "value": pool},
            {"type": "sym", "value": "constant"},
            {"type": "bytes", "value": "suAvz8pslvitXL2E53hKd3s22clqJFlALE9FhGKqt/A="},
            {"type": "vec", "value": [{"type": "u32", "value": 10}]}
        ]}),
        event_index: 0,
        op_index: None,
        stage: None,
        ledger_sequence: 10,
        created_at: 1_700_000_000,
    }
}

#[cfg(test)]
fn pool_instance_declaring(
    pool: &str,
    router: &str,
) -> xdr_parser::pool_state::ExtractedPoolInstance {
    xdr_parser::pool_state::ExtractedPoolInstance {
        state: xdr_parser::pool_state::PoolInstanceState {
            pool: pool.into(),
            token_share: None,
            total_shares: None,
            plane: Some("CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY".into()),
            router: Some(router.into()),
            reserves: Vec::new(),
        },
        ledger_sequence: 10,
        created: true,
    }
}

#[cfg(test)]
fn stage_registration(
    ledger: &ExtractedLedger,
    tx: &ExtractedTransaction,
    events: &[(String, Vec<ExtractedEvent>)],
    instances: &[xdr_parser::pool_state::ExtractedPoolInstance],
) -> stage::StagedLedger {
    let writes: Vec<xdr_parser::pool_family::PoolFamilyWrite> = instances
        .iter()
        .cloned()
        .map(xdr_parser::pool_family::PoolFamilyWrite::RouterPool)
        .collect();
    stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger,
        transactions: std::slice::from_ref(tx),
        operations: &[(tx.hash.clone(), vec![])],
        events,
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &writes,
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare")
}

/// Two writers feed `pool_state_changes` — the plane arm and the
/// concentrated-instance arm — and they collide on a pool's registration
/// ledger. Emitting both would leave the winner to a version-less RMT's
/// arbitrary intra-ledger pick (the defect class task 0463 measured on
/// `balances`). Staging folds them to ONE row per (pool, ledger) — the same
/// mechanism the classic twin `liquidity_pool_snapshots` uses.
#[test]
fn two_writers_for_one_pool_and_ledger_fold_to_one_row() {
    use xdr_parser::pool_state::{
        ExtractedPlanePoolData, ExtractedPoolInstance, PlanePoolData, PoolInstanceState,
    };
    const POOL: &str = "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH";
    const PLANE: &str = "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY";

    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x74);
    // Same pool, same ledger, both arms: the plane writes it, and the pool's
    // own instance also carries Reserve0/Reserve1.
    let plane_write = ExtractedPlanePoolData {
        data: PlanePoolData {
            plane: PLANE.into(),
            pool: POOL.into(),
            reserves: vec!["100".into(), "200".into()],
        },
        ledger_sequence: 10,
    };
    let instance = ExtractedPoolInstance {
        state: PoolInstanceState {
            pool: POOL.into(),
            token_share: None,
            total_shares: None,
            plane: Some(PLANE.into()),
            router: Some("CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK".into()),
            reserves: vec!["777".into(), "888".into()],
        },
        ledger_sequence: 10,
        created: true,
    };

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[
            xdr_parser::pool_family::PoolFamilyWrite::RouterPlane(plane_write.clone()),
            xdr_parser::pool_family::PoolFamilyWrite::RouterPool(instance.clone()),
        ],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare");

    assert_eq!(
        staged.pool_state_change_rows.len(),
        1,
        "one (pool, ledger) = one row, whichever writers produced it"
    );
    let row = &staged.pool_state_change_rows[0];
    assert_eq!(
        row.reserves,
        vec![777i128, 888i128],
        "the instance arm runs second and is the more specific source"
    );
}

/// The registration names its pool in an attacker-chosen DATA payload, so it
/// only becomes a row when the NAMED POOL declares that emitter as its router
/// in its own (ledger-authenticated) instance storage. Review #438: without
/// this, any contract could emit `add_pool` naming a REAL pool and replace its
/// registry row wholesale — `liquidity_pools` is RMT keyed on `pool_id`, so a
/// later ledger wins outright.
#[test]
fn prepare_refuses_a_registration_the_pool_does_not_corroborate() {
    const VICTIM: &str = "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH";
    const REAL_ROUTER: &str = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK";
    const ATTACKER: &str = "CDTSSTLKVVPWJZXVCGJJNGWKH5MY7OMINVXTB7DGFMDJTCCDBCSRG52O";

    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x71);
    let events = vec![(
        tx.hash.clone(),
        vec![add_pool_event(
            &tx.hash,
            ATTACKER,
            VICTIM,
            EventSource::TxLevel,
        )],
    )];
    // The victim's own instance names its REAL router, not the attacker.
    let instances = [pool_instance_declaring(VICTIM, REAL_ROUTER)];

    let staged = stage_registration(&ledger, &tx, &events, &instances);

    assert!(
        staged.pool_rows.iter().all(|r| r.pool_kind == 0),
        "a registration the named pool does not corroborate must not become a row"
    );
}

/// The corroborated case still registers — the guard must not cost real pools.
#[test]
fn prepare_accepts_a_registration_the_pool_corroborates() {
    const POOL: &str = "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH";
    const ROUTER: &str = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK";

    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x72);
    let events = vec![(
        tx.hash.clone(),
        vec![add_pool_event(&tx.hash, ROUTER, POOL, EventSource::TxLevel)],
    )];
    let instances = [pool_instance_declaring(POOL, ROUTER)];

    let staged = stage_registration(&ledger, &tx, &events, &instances);

    let row = staged
        .pool_rows
        .iter()
        .find(|r| r.pool_kind == 1)
        .expect("the corroborated registration stages a registry row");
    assert_eq!(row.deployment_id, ids::contract_id(ROUTER));
    assert_eq!(row.fee_bps, 10);
}

/// A pool from an OLDER deployment declares no `Router` (five of the ten
/// deployments on chain, 23 pools — read 2026-09-01). It cannot be
/// corroborated, because the chain never recorded who registered it, so it is
/// ACCEPTED rather than dropped: a missing key is an older contract version,
/// not a forgery. Its instance state must still stage, since `plane_id` is
/// what makes its reserves readable at all.
#[test]
fn a_registration_for_a_pool_that_declares_no_router_is_accepted() {
    const POOL: &str = "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH";
    const ROUTER: &str = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK";
    const PLANE: &str = "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY";

    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x75);
    let events = vec![(
        tx.hash.clone(),
        vec![add_pool_event(&tx.hash, ROUTER, POOL, EventSource::TxLevel)],
    )];
    let mut legacy = pool_instance_declaring(POOL, ROUTER);
    legacy.state.router = None; // the older contract shape
    let instances = [legacy];

    let staged = stage_registration(&ledger, &tx, &events, &instances);

    assert!(
        staged.pool_rows.iter().any(|r| r.pool_kind == 1),
        "an unverifiable pool is still a real pool — it must not be dropped"
    );
    let inst = staged
        .pool_instance_state_rows
        .iter()
        .find(|r| r.plane_id == ids::contract_id(PLANE))
        .expect("its plane must stage — reserve reads filter on it");
    assert_eq!(inst.plane_id, ids::contract_id(PLANE));
}

/// The induced-forgery signature (decision karolkow 2026-09-02): a
/// router-less pool whose instance was only TOUCHED this ledger — not
/// created — is exactly what an attacker produces by poking an existing
/// victim pool in the same transaction as a forged `add_pool`. No genuine
/// registration ever looks like this (497/497 measured registrations create
/// the instance in the registering transaction), so the registration is
/// refused. The instance STATE still stages — the declaration is authentic
/// regardless of who emitted the event.
#[test]
fn a_routerless_registration_with_a_merely_touched_instance_is_refused() {
    const VICTIM: &str = "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH";
    const ATTACKER: &str = "CDTSSTLKVVPWJZXVCGJJNGWKH5MY7OMINVXTB7DGFMDJTCCDBCSRG52O";
    const PLANE: &str = "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY";

    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x76);
    let events = vec![(
        tx.hash.clone(),
        vec![add_pool_event(
            &tx.hash,
            ATTACKER,
            VICTIM,
            EventSource::TxLevel,
        )],
    )];
    let mut touched = pool_instance_declaring(VICTIM, ATTACKER);
    touched.state.router = None; // the older, router-less contract shape
    touched.created = false; // an UPDATE — the victim already existed
    let instances = [touched];

    let staged = stage_registration(&ledger, &tx, &events, &instances);

    assert!(
        staged.pool_rows.iter().all(|r| r.pool_kind == 0),
        "a touched-not-created router-less registration is the forgery \
         signature and must not become a registry row"
    );
    assert!(
        staged
            .pool_instance_state_rows
            .iter()
            .any(|r| r.plane_id == ids::contract_id(PLANE)),
        "the pool's own declaration still stages — it is authentic state"
    );
}

/// The diagnostic container carries copies of events from FAILED transactions
/// (task 0182). A registration that never applied must not become a pool —
/// every sibling detector filters this container; review #438 found this one
/// missing the guard.
#[test]
fn prepare_ignores_a_registration_from_the_diagnostic_container() {
    const POOL: &str = "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH";
    const ROUTER: &str = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK";

    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x73);
    let events = vec![(
        tx.hash.clone(),
        vec![add_pool_event(
            &tx.hash,
            ROUTER,
            POOL,
            EventSource::Diagnostic,
        )],
    )];
    // Fully corroborated — ONLY the event source may keep this out.
    let instances = [pool_instance_declaring(POOL, ROUTER)];

    let staged = stage_registration(&ledger, &tx, &events, &instances);

    assert!(
        staged.pool_rows.iter().all(|r| r.pool_kind == 0),
        "a diagnostic-container registration must not become a row"
    );
}

#[test]
fn prepare_stages_plane_writes_and_instance_share_tokens() {
    // Real values end to end: the plane write and instance from registration
    // ledger 63,893,403 (the raw-ledger test's ground truth), through the
    // full staging pass.
    use xdr_parser::pool_state::{
        ExtractedPlanePoolData, ExtractedPoolInstance, PlanePoolData, PoolInstanceState,
    };
    const POOL: &str = "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH";
    const PLANE: &str = "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY";
    const SHARE: &str = "CC5PU23MKXHUFJKGG5FAUG7MFZX2KMWXPNZP26DDYW76VCB26UWMPEI6";

    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x64);
    let plane_write = ExtractedPlanePoolData {
        data: PlanePoolData {
            plane: PLANE.into(),
            pool: POOL.into(),
            reserves: vec!["100000000000".into(), "30617317".into()],
        },
        ledger_sequence: 10,
    };
    let instance = ExtractedPoolInstance {
        state: PoolInstanceState {
            pool: POOL.into(),
            token_share: Some(SHARE.into()),
            total_shares: None,
            plane: Some(PLANE.into()),
            router: Some("CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK".into()),
            reserves: Vec::new(),
        },
        ledger_sequence: 10,
        created: true,
    };
    // A concentrated-style instance (no share token) stages its PLANE, with
    // the structural share_token_id = 0.
    let conc = ExtractedPoolInstance {
        state: PoolInstanceState {
            pool: "CC642QYWXXR2HUZDNJ6KYN5LV5JFPFPT4Q6YNKLZLYEFWZZZ5SJYLA5G".into(),
            token_share: None,
            total_shares: None,
            plane: Some(PLANE.into()),
            router: Some("CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK".into()),
            // Real values from the hot-ledger probe: concentrated reserves
            // ride the INSTANCE, and must stage a snapshot row.
            reserves: vec!["4112908590".into(), "250000000000".into()],
        },
        ledger_sequence: 10,
        created: true,
    };

    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger: &ledger,
        transactions: std::slice::from_ref(&tx),
        operations: &[(tx.hash.clone(), vec![])],
        events: &[],
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &[
            xdr_parser::pool_family::PoolFamilyWrite::RouterPlane(plane_write.clone()),
            xdr_parser::pool_family::PoolFamilyWrite::RouterPool(instance),
            xdr_parser::pool_family::PoolFamilyWrite::RouterPool(conc),
        ],
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare");

    assert_eq!(
        staged.pool_state_change_rows.len(),
        2,
        "one plane-sourced (fungible) + one instance-sourced (concentrated)"
    );
    // Rows are distinguished by their reserve VALUES — the (pool, ledger)
    // grain carries no intra-ledger fields any more (parse-time collapse).
    let conc_snap = staged
        .pool_state_change_rows
        .iter()
        .find(|r| r.reserves == vec![4112908590i128, 250000000000i128])
        .expect("concentrated snapshot from the instance");
    let snap = staged
        .pool_state_change_rows
        .iter()
        .find(|r| r.reserves == vec![100000000000i128, 30617317i128])
        .expect("fungible snapshot from the plane");
    assert_eq!(snap.plane_id, ids::contract_id(PLANE));
    assert_ne!(
        conc_snap.pool_id, snap.pool_id,
        "the two rows belong to two different pools"
    );

    // BOTH instances stage a row: `plane_id` is the reserve-provenance
    // authority and every pool declares one. Only the fungible pool carries a
    // share token; the concentrated one keeps the structural 0.
    assert_eq!(
        staged.pool_instance_state_rows.len(),
        2,
        "every pool instance declares a plane, so every one stages a row"
    );
    let fungible = staged
        .pool_instance_state_rows
        .iter()
        .find(|r| r.share_token_id != 0)
        .expect("the fungible instance stages its share token");
    assert_eq!(fungible.share_token_id, ids::contract_id(SHARE));
    assert!(
        staged
            .pool_instance_state_rows
            .iter()
            .all(|r| r.plane_id != 0),
        "plane_id is load-bearing for reserve provenance — never 0"
    );
}

// ---------------------------------------------------------------------------
// Soroswap staging (task 0518): new_pair corroboration + the pair's own
// instance as reserve source and declaration. Addresses are real mainnet
// ones (the first pair in history + the documented factory).
// ---------------------------------------------------------------------------

#[cfg(test)]
const SORO_FACTORY: &str = "CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2";
#[cfg(test)]
const SORO_PAIR: &str = "CDMC44BMEGF5GMJHNP6NQA3LLBMWLONQFV37E2J5NWYYBBEXNMYMKRBO";
#[cfg(test)]
const SORO_T0: &str = "CAINX4EAMVB5DJLM3TP7Z5AZIKEYBA6LKURSBF75C6MS35NDY3FYLV6Y";
#[cfg(test)]
const SORO_T1: &str = "CAVXDPJ2M6BWRVTJ3VOVSE3U7QISFS4ET3XA3ONS3UD47X6TA54PIXFJ";

#[cfg(test)]
fn new_pair_event(tx_hash: &str, factory: &str, pair: &str) -> ExtractedEvent {
    ExtractedEvent {
        transaction_hash: tx_hash.to_string(),
        event_type: ContractEventType::Contract,
        source: EventSource::TxLevel,
        contract_id: Some(factory.to_string()),
        topics: serde_json::json!([
            {"type": "string", "value": "SoroswapFactory"},
            {"type": "sym", "value": "new_pair"}
        ]),
        data: serde_json::json!({"type": "map", "value": [
            {"key": {"type": "sym", "value": "new_pairs_length"},
             "value": {"type": "u32", "value": 1}},
            {"key": {"type": "sym", "value": "pair"}, "value": {"type": "address", "value": pair}},
            {"key": {"type": "sym", "value": "token_0"}, "value": {"type": "address", "value": SORO_T0}},
            {"key": {"type": "sym", "value": "token_1"}, "value": {"type": "address", "value": SORO_T1}}
        ]}),
        ledger_sequence: 10,
        event_index: 0,
        op_index: None,
        stage: None,
        created_at: 1_700_000_000,
    }
}

#[cfg(test)]
fn factory_pair_instance(
    reserves: Option<(&str, &str)>,
    total_supply: Option<&str>,
    created: bool,
) -> xdr_parser::pool_pair_factory::ExtractedFactoryPair {
    xdr_parser::pool_pair_factory::ExtractedFactoryPair {
        state: xdr_parser::pool_pair_factory::FactoryPairState {
            pair: SORO_PAIR.into(),
            token_0: SORO_T0.into(),
            token_1: SORO_T1.into(),
            factory: SORO_FACTORY.into(),
            reserves: reserves.map(|(a, b)| (a.to_string(), b.to_string())),
            total_supply: total_supply.map(str::to_string),
        },
        ledger_sequence: 10,
        created,
    }
}

#[cfg(test)]
fn stage_factory_pair(
    ledger: &ExtractedLedger,
    tx: &ExtractedTransaction,
    events: &[(String, Vec<ExtractedEvent>)],
    pairs: &[xdr_parser::pool_pair_factory::ExtractedFactoryPair],
) -> stage::StagedLedger {
    let writes: Vec<xdr_parser::pool_family::PoolFamilyWrite> = pairs
        .iter()
        .cloned()
        .map(xdr_parser::pool_family::PoolFamilyWrite::FactoryPair)
        .collect();
    stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger,
        transactions: std::slice::from_ref(tx),
        operations: &[(tx.hash.clone(), vec![])],
        events,
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &writes,
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("prepare")
}

/// A corroborated, created-in-ledger registration stages the full registry
/// row: kind 1, both leg tokens, the factory as deployment, the vendor's
/// compiled-in 30 bps fee, and an EMPTY pool_type_raw (decision 64).
#[test]
fn a_corroborated_new_pair_registers_with_the_pairs_own_facts() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x81);
    let events = vec![(
        tx.hash.clone(),
        vec![new_pair_event(&tx.hash, SORO_FACTORY, SORO_PAIR)],
    )];
    let pairs = [factory_pair_instance(None, None, true)];

    let staged = stage_factory_pair(&ledger, &tx, &events, &pairs);

    let row = staged
        .pool_rows
        .iter()
        .find(|r| r.pool_kind == 1)
        .expect("the corroborated registration stages a registry row");
    assert_eq!(row.deployment_id, ids::contract_id(SORO_FACTORY));
    assert_eq!(
        row.legs,
        vec![ids::contract_id(SORO_T0), ids::contract_id(SORO_T1)]
    );
    assert_eq!(row.fee_bps, 30);
    assert_eq!(row.pool_type_raw, "");
    // A newborn pair has no reserves yet — no state row, but the
    // declaration (self plane, self share token) MUST stage.
    assert!(staged.pool_state_change_rows.is_empty());
    let decl = staged
        .pool_instance_state_rows
        .first()
        .expect("the declaration stages at birth");
    assert_eq!(decl.plane_id, ids::contract_id(SORO_PAIR));
    assert_eq!(decl.share_token_id, ids::contract_id(SORO_PAIR));
    assert_eq!(decl.total_shares, 0);
}

/// The two forgery signatures: an emitter the pair does not declare, and a
/// pair instance that was merely TOUCHED (not created) this ledger.
#[test]
fn uncorroborated_or_touched_new_pairs_are_refused() {
    const ATTACKER: &str = "CDTSSTLKVVPWJZXVCGJJNGWKH5MY7OMINVXTB7DGFMDJTCCDBCSRG52O";
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x82);
    let events = vec![(
        tx.hash.clone(),
        vec![new_pair_event(&tx.hash, ATTACKER, SORO_PAIR)],
    )];
    // The pair's instance names the REAL factory — the attacker's event
    // must not become a row.
    let pairs = [factory_pair_instance(None, None, true)];
    let staged = stage_factory_pair(&ledger, &tx, &events, &pairs);
    assert!(staged.pool_rows.iter().all(|r| r.pool_kind == 0));

    // Genuine factory as emitter, but the instance was only touched — the
    // induced-forgery signature. Refused the same way.
    let tx2 = synthetic_tx(0x83);
    let events2 = vec![(
        tx2.hash.clone(),
        vec![new_pair_event(&tx2.hash, SORO_FACTORY, SORO_PAIR)],
    )];
    let touched = [factory_pair_instance(None, None, false)];
    let staged2 = stage_factory_pair(&ledger, &tx2, &events2, &touched);
    assert!(staged2.pool_rows.iter().all(|r| r.pool_kind == 0));
}

/// A live pair write stages the reserve row stamped with the pair's OWN id
/// as plane (owner, stamp and declaration coincide) plus the refreshed
/// declaration carrying the LP-token supply.
#[test]
fn a_pair_write_stages_self_stamped_reserves_and_supply() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x84);
    let pairs = [factory_pair_instance(
        Some(("3362421101426", "585980063616")),
        Some("1387420389"),
        false,
    )];

    let staged = stage_factory_pair(&ledger, &tx, &[], &pairs);

    let state = staged
        .pool_state_change_rows
        .first()
        .expect("reserves stage from the pair's own instance");
    assert_eq!(
        state.reserves,
        vec![3_362_421_101_426_i128, 585_980_063_616]
    );
    assert_eq!(
        state.plane_id,
        ids::contract_id(SORO_PAIR),
        "the stamp is the pair itself — the provenance filter passes by construction"
    );
    let decl = staged
        .pool_instance_state_rows
        .first()
        .expect("declaration");
    assert_eq!(decl.total_shares, 1_387_420_389);
    assert_eq!(decl.share_token_id, ids::contract_id(SORO_PAIR));
}

// ---------------------------------------------------------------------------
// Config-factory staging (task 0518, third adapter): create/liquidity_pool
// corroboration via the pool's OWN full CONFIG + created gate, keyed
// persistent entries as reserve source, and the instance-row-only-on-config
// clobber protection. Addresses are real mainnet ones (the Phoenix-family
// factory + its newest pool, creation ledger 64,030,567).
// ---------------------------------------------------------------------------

#[cfg(test)]
const CFG_FACTORY: &str = "CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI";
#[cfg(test)]
const CFG_POOL: &str = "CCPPPTDWJIWXQUQ2CN64S5JYQ7GYWVZIT7YWUUTH75HKIZX53Z2CE3XI";
#[cfg(test)]
const CFG_TA: &str = "CBZ7M5B3Y4WWBZ5XK5UZCAFOEZ23KSSZXYECYX3IXM6E2JOLQC52DK32";
#[cfg(test)]
const CFG_TB: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";
#[cfg(test)]
const CFG_SHARE: &str = "CA3KLIRAM6BKPN6BPPKTDX3CSY2DSM4YZAX54KZLER25X2QRK3FGDXR6";

#[cfg(test)]
fn liquidity_pool_created_event(tx_hash: &str, factory: &str, pool: &str) -> ExtractedEvent {
    ExtractedEvent {
        transaction_hash: tx_hash.to_string(),
        event_type: ContractEventType::Contract,
        source: EventSource::TxLevel,
        contract_id: Some(factory.to_string()),
        topics: serde_json::json!([
            {"type": "string", "value": "create"},
            {"type": "string", "value": "liquidity_pool"}
        ]),
        data: serde_json::json!({"type": "address", "value": pool}),
        ledger_sequence: 10,
        event_index: 0,
        op_index: None,
        stage: None,
        created_at: 1_700_000_000,
    }
}

#[cfg(test)]
fn config_pool_write(
    with_config: bool,
    reserves: Option<(&str, &str)>,
    total_shares: Option<&str>,
    created: bool,
) -> xdr_parser::pool_config_factory::ExtractedConfigPool {
    xdr_parser::pool_config_factory::ExtractedConfigPool {
        state: xdr_parser::pool_config_factory::ConfigPoolState {
            pool: CFG_POOL.into(),
            config: with_config.then(|| xdr_parser::pool_config_factory::PoolConfig {
                token_a: CFG_TA.into(),
                token_b: CFG_TB.into(),
                share_token: CFG_SHARE.into(),
                pool_type: 0,
                total_fee_bps: 50,
            }),
            reserves: reserves.map(|(a, b)| (a.to_string(), b.to_string())),
            total_shares: total_shares.map(str::to_string),
        },
        ledger_sequence: 10,
        created,
    }
}

#[cfg(test)]
/// An emitter's own membership-list write — stage 1 of the registration
/// gate.
#[cfg(test)]
fn address_list(
    owner: &str,
    members: &[&str],
) -> xdr_parser::pool_config_factory::ExtractedAddressListWrite {
    xdr_parser::pool_config_factory::ExtractedAddressListWrite {
        owner: owner.into(),
        members: members.iter().map(|m| m.to_string()).collect(),
        ledger_sequence: 10,
    }
}

#[cfg(test)]
fn stage_config_pool(
    ledger: &ExtractedLedger,
    tx: &ExtractedTransaction,
    events: &[(String, Vec<ExtractedEvent>)],
    pools: &[xdr_parser::pool_config_factory::ExtractedConfigPool],
    lists: &[xdr_parser::pool_config_factory::ExtractedAddressListWrite],
) -> stage::StagedLedger {
    let writes: Vec<xdr_parser::pool_family::PoolFamilyWrite> = pools
        .iter()
        .cloned()
        .map(xdr_parser::pool_family::PoolFamilyWrite::ConfigPool)
        .chain(
            lists
                .iter()
                .cloned()
                .map(xdr_parser::pool_family::PoolFamilyWrite::AddressList),
        )
        .collect();
    stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger,
        transactions: std::slice::from_ref(tx),
        operations: &[(tx.hash.clone(), vec![])],
        events,
        invocations: &[],
        contract_interfaces: &[],
        contract_deployments: &[],
        account_states: &[],
        liquidity_pools: &[],
        pool_snapshots: &[],
        assets: &[],
        nfts: &[],
        nft_events: &[],
        lp_positions: &[],
        contract_metadata_writes: &[],
        soroban_token_balances: &[],
        pool_family_writes: &writes,
        sac_classic: &std::collections::HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &std::collections::HashMap::new(),
        prior_contract_verdicts: &std::collections::HashMap::new(),
        prior_contract_rows: &std::collections::HashMap::new(),
    })
    .expect("staging succeeds — refusals must not fail the ledger")
}

/// The genuine creation signature: event + the pool's own CONFIG + created
/// instance, all in one ledger. Every registry fact comes from the CONFIG.
#[test]
fn a_corroborated_liquidity_pool_registers_with_the_pools_own_config() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x85);
    let events = vec![(
        tx.hash.clone(),
        vec![liquidity_pool_created_event(
            &tx.hash,
            CFG_FACTORY,
            CFG_POOL,
        )],
    )];
    // The creation transaction writes config, zero reserves and zero shares.
    let pools = [config_pool_write(true, Some(("0", "0")), Some("0"), true)];

    let lists = [address_list(CFG_FACTORY, &[CFG_POOL])];
    let staged = stage_config_pool(&ledger, &tx, &events, &pools, &lists);

    let row = staged
        .pool_rows
        .iter()
        .find(|r| r.pool_kind == 1)
        .expect("the corroborated registration stages a registry row");
    assert_eq!(row.deployment_id, ids::contract_id(CFG_FACTORY));
    assert_eq!(
        row.legs,
        vec![ids::contract_id(CFG_TA), ids::contract_id(CFG_TB)]
    );
    assert_eq!(row.fee_bps, 50, "the per-pool fee comes from CONFIG");
    assert_eq!(
        row.pool_type_raw, "0",
        "the PairType discriminant, verbatim"
    );
    // Creation stages the TRUE-zero reserve pair and the declaration with
    // the SEPARATE share token.
    let state = staged.pool_state_change_rows.first().expect("reserve row");
    assert_eq!(state.reserves, vec![0_i128, 0]);
    assert_eq!(state.plane_id, ids::contract_id(CFG_POOL));
    let decl = staged
        .pool_instance_state_rows
        .first()
        .expect("the declaration stages at birth");
    assert_eq!(decl.plane_id, ids::contract_id(CFG_POOL));
    assert_eq!(decl.share_token_id, ids::contract_id(CFG_SHARE));
    assert_eq!(decl.total_shares, 0);
}

/// The two refusal signatures: a named pool that never wrote its CONFIG
/// this ledger, and one whose entries were written without an instance
/// CREATION (the induced-forgery signature).
#[test]
fn unconfigured_or_touched_liquidity_pools_are_refused() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x86);
    let events = vec![(
        tx.hash.clone(),
        vec![liquidity_pool_created_event(
            &tx.hash,
            CFG_FACTORY,
            CFG_POOL,
        )],
    )];
    // Reserves only, no CONFIG — the shape never fully declared itself.
    let unconfigured = [config_pool_write(false, Some(("1", "2")), None, true)];
    let lists = [address_list(CFG_FACTORY, &[CFG_POOL])];
    let staged = stage_config_pool(&ledger, &tx, &events, &unconfigured, &lists);
    assert!(staged.pool_rows.iter().all(|r| r.pool_kind == 0));

    // Full CONFIG but the instance was only TOUCHED, not created.
    let tx2 = synthetic_tx(0x87);
    let events2 = vec![(
        tx2.hash.clone(),
        vec![liquidity_pool_created_event(
            &tx2.hash,
            CFG_FACTORY,
            CFG_POOL,
        )],
    )];
    let touched = [config_pool_write(true, None, None, false)];
    let staged2 = stage_config_pool(&ledger, &tx2, &events2, &touched, &lists);
    assert!(staged2.pool_rows.iter().all(|r| r.pool_kind == 0));
}

/// A per-operation write (reserves + TotalShares, NO config) stages the
/// self-stamped reserve row and NOTHING into `pool_instance_state` — the
/// table is RMT whole-row keyed on pool_id, and a config-less row would
/// clobber `share_token_id` to 0.
#[test]
fn a_config_pool_operation_stages_reserves_without_clobbering_the_declaration() {
    let ledger = synthetic_ledger();
    let tx = synthetic_tx(0x88);
    let pools = [config_pool_write(
        false,
        Some(("123456789", "987654321")),
        Some("55555"),
        false,
    )];

    let staged = stage_config_pool(&ledger, &tx, &[], &pools, &[]);

    let state = staged
        .pool_state_change_rows
        .first()
        .expect("reserves stage from the pool's own keyed entries");
    assert_eq!(state.reserves, vec![123_456_789_i128, 987_654_321]);
    assert_eq!(
        state.plane_id,
        ids::contract_id(CFG_POOL),
        "the stamp is the pool itself — the provenance filter passes by construction"
    );
    assert!(
        staged.pool_instance_state_rows.is_empty(),
        "a config-less write must NOT stage an instance row — RMT whole-row \
         replace would zero the share token"
    );
}

/// The third forgery shape (review #447), against the TWO-STAGE gate.
///
/// Stage 1 (membership list, pointwise): an event-only co-claimer is
/// refused alone and the GENUINE registration survives. Stage 2
/// (conflict): an attacker who also writes the pool into his OWN list
/// leaves two corroborated claimants — no ledger fact arbitrates them, so
/// BOTH refuse rather than let the RMT version tie pick one.
#[test]
fn config_pool_gate_is_pointwise_first_and_refuses_both_on_true_conflict() {
    const ATTACKER: &str = "CDTSSTLKVVPWJZXVCGJJNGWKH5MY7OMINVXTB7DGFMDJTCCDBCSRG52O";
    let ledger = synthetic_ledger();
    // The pool itself is genuine: created, full CONFIG — the gate the
    // attacker piggybacks on.
    let pools = [config_pool_write(true, Some(("0", "0")), Some("0"), true)];

    // Stage 1: attacker emits but records nothing in his own storage —
    // refused alone, the genuine row STAGES with the genuine attribution.
    let tx = synthetic_tx(0x89);
    let events = vec![(
        tx.hash.clone(),
        vec![
            liquidity_pool_created_event(&tx.hash, CFG_FACTORY, CFG_POOL),
            liquidity_pool_created_event(&tx.hash, ATTACKER, CFG_POOL),
        ],
    )];
    let genuine_list = [address_list(CFG_FACTORY, &[CFG_POOL])];
    let staged = stage_config_pool(&ledger, &tx, &events, &pools, &genuine_list);
    let rows: Vec<_> = staged
        .pool_rows
        .iter()
        .filter(|r| r.pool_kind == 1)
        .collect();
    assert_eq!(rows.len(), 1, "the genuine registration survives pointwise");
    assert_eq!(
        rows[0].deployment_id,
        ids::contract_id(CFG_FACTORY),
        "attribution is the emitter that corroborated in its own storage"
    );

    // Stage 2: the attacker also writes the pool into HIS own list — two
    // corroborated claimants, refuse BOTH.
    let tx2 = synthetic_tx(0x8a);
    let events2 = vec![(
        tx2.hash.clone(),
        vec![
            liquidity_pool_created_event(&tx2.hash, CFG_FACTORY, CFG_POOL),
            liquidity_pool_created_event(&tx2.hash, ATTACKER, CFG_POOL),
        ],
    )];
    let both_lists = [
        address_list(CFG_FACTORY, &[CFG_POOL]),
        address_list(ATTACKER, &[CFG_POOL]),
    ];
    let staged2 = stage_config_pool(&ledger, &tx2, &events2, &pools, &both_lists);
    assert!(
        staged2.pool_rows.iter().all(|r| r.pool_kind == 0),
        "two corroborated claimants must refuse BOTH rows — a nondeterministic \
         deployment_id is worse than a loud gap"
    );

    // An identical duplicate from ONE corroborated emitter collapses to a
    // single row.
    let tx3 = synthetic_tx(0x8b);
    let events3 = vec![(
        tx3.hash.clone(),
        vec![
            liquidity_pool_created_event(&tx3.hash, CFG_FACTORY, CFG_POOL),
            liquidity_pool_created_event(&tx3.hash, CFG_FACTORY, CFG_POOL),
        ],
    )];
    let staged3 = stage_config_pool(&ledger, &tx3, &events3, &pools, &genuine_list);
    assert_eq!(
        staged3
            .pool_rows
            .iter()
            .filter(|r| r.pool_kind == 1)
            .count(),
        1,
        "one emitter announcing twice stages exactly one row"
    );
}
