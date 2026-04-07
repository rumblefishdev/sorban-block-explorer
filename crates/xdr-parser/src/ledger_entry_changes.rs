//! LedgerEntryChanges extraction from transaction metadata.
//!
//! Extracts all ledger entry mutations (created, updated, removed, state)
//! from `TransactionMeta` V3/V4. Each `LedgerEntryChange` is converted into
//! an `ExtractedLedgerEntryChange` with typed key fields and full entry data.
//!
//! Supported entry types: account, trustline, offer, data, claimable_balance,
//! liquidity_pool, contract_data, contract_code, config_setting, ttl.

use serde_json::{Value, json};
use stellar_xdr::curr::*;

use crate::scval::scval_to_typed_json;
use crate::types::ExtractedLedgerEntryChange;

/// Extract all ledger entry changes from a transaction's metadata.
///
/// Iterates `tx_changes_before`, per-operation changes, and `tx_changes_after`
/// in order. Returns one `ExtractedLedgerEntryChange` per `LedgerEntryChange`.
/// Non-V3/V4 meta produces an empty vec.
pub fn extract_ledger_entry_changes(
    tx_meta: &TransactionMeta,
    transaction_hash: &str,
    ledger_sequence: u32,
    created_at: i64,
) -> Vec<ExtractedLedgerEntryChange> {
    let mut results = Vec::new();
    let mut index: u32 = 0;

    match tx_meta {
        TransactionMeta::V3(v3) => {
            extract_from_changes(
                &v3.tx_changes_before,
                None,
                transaction_hash,
                ledger_sequence,
                created_at,
                &mut index,
                &mut results,
            );
            for (op_idx, op_meta) in v3.operations.iter().enumerate() {
                let op_index =
                    Some(u32::try_from(op_idx).expect("operation index does not fit into u32"));
                extract_from_changes(
                    &op_meta.changes,
                    op_index,
                    transaction_hash,
                    ledger_sequence,
                    created_at,
                    &mut index,
                    &mut results,
                );
            }
            extract_from_changes(
                &v3.tx_changes_after,
                None,
                transaction_hash,
                ledger_sequence,
                created_at,
                &mut index,
                &mut results,
            );
        }
        TransactionMeta::V4(v4) => {
            extract_from_changes(
                &v4.tx_changes_before,
                None,
                transaction_hash,
                ledger_sequence,
                created_at,
                &mut index,
                &mut results,
            );
            for (op_idx, op_meta) in v4.operations.iter().enumerate() {
                let op_index =
                    Some(u32::try_from(op_idx).expect("operation index does not fit into u32"));
                extract_from_changes(
                    &op_meta.changes,
                    op_index,
                    transaction_hash,
                    ledger_sequence,
                    created_at,
                    &mut index,
                    &mut results,
                );
            }
            extract_from_changes(
                &v4.tx_changes_after,
                None,
                transaction_hash,
                ledger_sequence,
                created_at,
                &mut index,
                &mut results,
            );
        }
        _ => {}
    }

    results
}

/// Process a single `LedgerEntryChanges` collection.
fn extract_from_changes(
    changes: &LedgerEntryChanges,
    operation_index: Option<u32>,
    transaction_hash: &str,
    ledger_sequence: u32,
    created_at: i64,
    index: &mut u32,
    results: &mut Vec<ExtractedLedgerEntryChange>,
) {
    for change in changes.iter() {
        if let Some(extracted) = extract_single_change(
            change,
            operation_index,
            transaction_hash,
            ledger_sequence,
            created_at,
            *index,
        ) {
            results.push(extracted);
        }
        *index += 1;
    }
}

/// Convert a single `LedgerEntryChange` into an `ExtractedLedgerEntryChange`.
fn extract_single_change(
    change: &LedgerEntryChange,
    operation_index: Option<u32>,
    transaction_hash: &str,
    ledger_sequence: u32,
    created_at: i64,
    change_index: u32,
) -> Option<ExtractedLedgerEntryChange> {
    let (change_type, entry_type, key, data) = match change {
        LedgerEntryChange::Created(entry) => {
            let (et, k, d) = extract_entry_info(entry);
            ("created", et, k, Some(d))
        }
        LedgerEntryChange::Updated(entry) => {
            let (et, k, d) = extract_entry_info(entry);
            ("updated", et, k, Some(d))
        }
        LedgerEntryChange::Removed(ledger_key) => {
            let (et, k) = extract_key_info(ledger_key);
            ("removed", et, k, None)
        }
        LedgerEntryChange::State(entry) => {
            let (et, k, d) = extract_entry_info(entry);
            ("state", et, k, Some(d))
        }
        LedgerEntryChange::Restored(entry) => {
            let (et, k, d) = extract_entry_info(entry);
            ("restored", et, k, Some(d))
        }
    };

    Some(ExtractedLedgerEntryChange {
        transaction_hash: transaction_hash.to_string(),
        change_type: change_type.to_string(),
        entry_type: entry_type.to_string(),
        key,
        data,
        change_index,
        operation_index,
        ledger_sequence,
        created_at,
    })
}

// ---------------------------------------------------------------------------
// Entry data extraction (for created/updated/state)
// ---------------------------------------------------------------------------

/// Extract entry type, key fields, and full data from a `LedgerEntry`.
fn extract_entry_info(entry: &LedgerEntry) -> (&'static str, Value, Value) {
    match &entry.data {
        LedgerEntryData::Account(a) => ("account", account_key(a), account_data(a)),
        LedgerEntryData::Trustline(t) => ("trustline", trustline_key(t), trustline_data(t)),
        LedgerEntryData::Offer(o) => ("offer", offer_key(o), offer_data(o)),
        LedgerEntryData::Data(d) => ("data", data_entry_key(d), data_entry_data(d)),
        LedgerEntryData::ClaimableBalance(cb) => (
            "claimable_balance",
            claimable_balance_key(cb),
            claimable_balance_data(cb),
        ),
        LedgerEntryData::LiquidityPool(lp) => (
            "liquidity_pool",
            liquidity_pool_key(lp),
            liquidity_pool_data(lp),
        ),
        LedgerEntryData::ContractData(cd) => (
            "contract_data",
            contract_data_key(cd),
            contract_data_data(cd),
        ),
        LedgerEntryData::ContractCode(cc) => (
            "contract_code",
            contract_code_key(cc),
            contract_code_data(cc),
        ),
        // Config setting payload intentionally excluded — protocol-internal, not exposed by explorer.
        // Each variant has a different inner type; serializing all is high effort, low value.
        LedgerEntryData::ConfigSetting(cs) => ("config_setting", config_setting_key(cs), json!({})),
        LedgerEntryData::Ttl(t) => ("ttl", ttl_key(t), ttl_data(t)),
    }
}

// ---------------------------------------------------------------------------
// Key extraction from LedgerKey (for removed changes)
// ---------------------------------------------------------------------------

/// Extract entry type and key fields from a `LedgerKey`.
fn extract_key_info(key: &LedgerKey) -> (&'static str, Value) {
    match key {
        LedgerKey::Account(k) => ("account", json!({ "account_id": k.account_id.to_string() })),
        LedgerKey::Trustline(k) => (
            "trustline",
            json!({
                "account_id": k.account_id.to_string(),
                "asset": format_trustline_asset_key(&k.asset),
            }),
        ),
        LedgerKey::Offer(k) => (
            "offer",
            json!({
                "seller_id": k.seller_id.to_string(),
                "offer_id": k.offer_id,
            }),
        ),
        LedgerKey::Data(k) => (
            "data",
            json!({
                "account_id": k.account_id.to_string(),
                "data_name": String::from_utf8_lossy(k.data_name.as_slice()).to_string(),
            }),
        ),
        LedgerKey::ClaimableBalance(k) => (
            "claimable_balance",
            json!({ "balance_id": format_claimable_balance_id(&k.balance_id) }),
        ),
        LedgerKey::LiquidityPool(k) => (
            "liquidity_pool",
            json!({ "pool_id": hex::encode(k.liquidity_pool_id.0.clone()) }),
        ),
        LedgerKey::ContractData(k) => (
            "contract_data",
            json!({
                "contract": k.contract.to_string(),
                "key": scval_to_typed_json(&k.key),
                "durability": format_durability(&k.durability),
            }),
        ),
        LedgerKey::ContractCode(k) => ("contract_code", json!({ "hash": hex::encode(k.hash.0) })),
        LedgerKey::ConfigSetting(k) => (
            "config_setting",
            json!({ "config_setting_id": format!("{:?}", k.config_setting_id) }),
        ),
        LedgerKey::Ttl(k) => ("ttl", json!({ "key_hash": hex::encode(k.key_hash.0) })),
    }
}

// ---------------------------------------------------------------------------
// Account
// ---------------------------------------------------------------------------

fn account_key(a: &AccountEntry) -> Value {
    json!({ "account_id": a.account_id.to_string() })
}

fn account_data(a: &AccountEntry) -> Value {
    json!({
        "account_id": a.account_id.to_string(),
        "balance": a.balance,
        "seq_num": i64::from(a.seq_num.clone()),
        "num_sub_entries": a.num_sub_entries,
        "home_domain": String::from_utf8_lossy(a.home_domain.as_slice()).to_string(),
        "thresholds": hex::encode(a.thresholds.0),
        "flags": a.flags,
    })
}

// ---------------------------------------------------------------------------
// Trustline
// ---------------------------------------------------------------------------

fn trustline_key(t: &TrustLineEntry) -> Value {
    json!({
        "account_id": t.account_id.to_string(),
        "asset": format_trustline_asset(&t.asset),
    })
}

fn trustline_data(t: &TrustLineEntry) -> Value {
    json!({
        "account_id": t.account_id.to_string(),
        "asset": format_trustline_asset(&t.asset),
        "balance": t.balance,
        "limit": t.limit,
        "flags": t.flags,
    })
}

fn format_trustline_asset(asset: &TrustLineAsset) -> Value {
    match asset {
        TrustLineAsset::Native => json!("native"),
        TrustLineAsset::CreditAlphanum4(a) => json!({
            "type": "credit_alphanum4",
            "code": String::from_utf8_lossy(a.asset_code.as_slice()).trim_end_matches('\0').to_string(),
            "issuer": a.issuer.to_string(),
        }),
        TrustLineAsset::CreditAlphanum12(a) => json!({
            "type": "credit_alphanum12",
            "code": String::from_utf8_lossy(a.asset_code.as_slice()).trim_end_matches('\0').to_string(),
            "issuer": a.issuer.to_string(),
        }),
        TrustLineAsset::PoolShare(pool_id) => {
            json!({ "type": "pool_share", "pool_id": hex::encode(pool_id.0.clone()) })
        }
    }
}

fn format_trustline_asset_key(asset: &TrustLineAsset) -> Value {
    format_trustline_asset(asset)
}

// ---------------------------------------------------------------------------
// Offer
// ---------------------------------------------------------------------------

fn offer_key(o: &OfferEntry) -> Value {
    json!({
        "seller_id": o.seller_id.to_string(),
        "offer_id": o.offer_id,
    })
}

fn offer_data(o: &OfferEntry) -> Value {
    json!({
        "seller_id": o.seller_id.to_string(),
        "offer_id": o.offer_id,
        "selling": format_asset(&o.selling),
        "buying": format_asset(&o.buying),
        "amount": o.amount,
        "price": { "n": o.price.n, "d": o.price.d },
        "flags": o.flags,
    })
}

fn format_asset(asset: &Asset) -> Value {
    match asset {
        Asset::Native => json!("native"),
        Asset::CreditAlphanum4(a) => json!({
            "type": "credit_alphanum4",
            "code": String::from_utf8_lossy(a.asset_code.as_slice()).trim_end_matches('\0').to_string(),
            "issuer": a.issuer.to_string(),
        }),
        Asset::CreditAlphanum12(a) => json!({
            "type": "credit_alphanum12",
            "code": String::from_utf8_lossy(a.asset_code.as_slice()).trim_end_matches('\0').to_string(),
            "issuer": a.issuer.to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Data entry
// ---------------------------------------------------------------------------

fn data_entry_key(d: &DataEntry) -> Value {
    json!({
        "account_id": d.account_id.to_string(),
        "data_name": String::from_utf8_lossy(d.data_name.as_slice()).to_string(),
    })
}

fn data_entry_data(d: &DataEntry) -> Value {
    json!({
        "account_id": d.account_id.to_string(),
        "data_name": String::from_utf8_lossy(d.data_name.as_slice()).to_string(),
        "data_value": hex::encode(d.data_value.as_slice()),
    })
}

// ---------------------------------------------------------------------------
// Claimable balance
// ---------------------------------------------------------------------------

fn claimable_balance_key(cb: &ClaimableBalanceEntry) -> Value {
    json!({ "balance_id": format_claimable_balance_id(&cb.balance_id) })
}

fn claimable_balance_data(cb: &ClaimableBalanceEntry) -> Value {
    json!({
        "balance_id": format_claimable_balance_id(&cb.balance_id),
        "asset": format_asset(&cb.asset),
        "amount": cb.amount,
        "claimants": cb.claimants.iter().map(format_claimant).collect::<Vec<_>>(),
    })
}

fn format_claimable_balance_id(id: &ClaimableBalanceId) -> String {
    match id {
        ClaimableBalanceId::ClaimableBalanceIdTypeV0(hash) => hex::encode(hash.0),
    }
}

fn format_claimant(c: &Claimant) -> Value {
    match c {
        Claimant::ClaimantTypeV0(v0) => json!({
            "destination": v0.destination.to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Liquidity pool
// ---------------------------------------------------------------------------

fn liquidity_pool_key(lp: &LiquidityPoolEntry) -> Value {
    json!({ "pool_id": hex::encode(lp.liquidity_pool_id.0.clone()) })
}

fn liquidity_pool_data(lp: &LiquidityPoolEntry) -> Value {
    match &lp.body {
        LiquidityPoolEntryBody::LiquidityPoolConstantProduct(cp) => json!({
            "pool_id": hex::encode(lp.liquidity_pool_id.0.clone()),
            "type": "constant_product",
            "params": {
                "asset_a": format_asset(&cp.params.asset_a),
                "asset_b": format_asset(&cp.params.asset_b),
                "fee": cp.params.fee,
            },
            "reserve_a": cp.reserve_a,
            "reserve_b": cp.reserve_b,
            "total_pool_shares": cp.total_pool_shares,
            "pool_shares_trust_line_count": cp.pool_shares_trust_line_count,
        }),
    }
}

// ---------------------------------------------------------------------------
// Contract data
// ---------------------------------------------------------------------------

fn contract_data_key(cd: &ContractDataEntry) -> Value {
    json!({
        "contract": cd.contract.to_string(),
        "key": scval_to_typed_json(&cd.key),
        "durability": format_durability(&cd.durability),
    })
}

fn contract_data_data(cd: &ContractDataEntry) -> Value {
    json!({
        "contract": cd.contract.to_string(),
        "key": scval_to_typed_json(&cd.key),
        "durability": format_durability(&cd.durability),
        "val": scval_to_typed_json(&cd.val),
    })
}

fn format_durability(d: &ContractDataDurability) -> &'static str {
    match d {
        ContractDataDurability::Temporary => "temporary",
        ContractDataDurability::Persistent => "persistent",
    }
}

// ---------------------------------------------------------------------------
// Contract code
// ---------------------------------------------------------------------------

fn contract_code_key(cc: &ContractCodeEntry) -> Value {
    json!({ "hash": hex::encode(cc.hash.0) })
}

fn contract_code_data(cc: &ContractCodeEntry) -> Value {
    json!({
        "hash": hex::encode(cc.hash.0),
        "code_byte_len": cc.code.as_slice().len(),
    })
}

// ---------------------------------------------------------------------------
// Config setting
// ---------------------------------------------------------------------------

fn config_setting_key(cs: &ConfigSettingEntry) -> Value {
    let id = match cs {
        ConfigSettingEntry::ContractMaxSizeBytes(_) => "contract_max_size_bytes",
        ConfigSettingEntry::ContractComputeV0(_) => "contract_compute_v0",
        ConfigSettingEntry::ContractLedgerCostV0(_) => "contract_ledger_cost_v0",
        ConfigSettingEntry::ContractHistoricalDataV0(_) => "contract_historical_data_v0",
        ConfigSettingEntry::ContractEventsV0(_) => "contract_events_v0",
        ConfigSettingEntry::ContractBandwidthV0(_) => "contract_bandwidth_v0",
        ConfigSettingEntry::ContractCostParamsCpuInstructions(_) => "contract_cost_params_cpu",
        ConfigSettingEntry::ContractCostParamsMemoryBytes(_) => "contract_cost_params_memory",
        ConfigSettingEntry::ContractDataKeySizeBytes(_) => "contract_data_key_size_bytes",
        ConfigSettingEntry::ContractDataEntrySizeBytes(_) => "contract_data_entry_size_bytes",
        ConfigSettingEntry::StateArchival(_) => "state_archival",
        ConfigSettingEntry::ContractExecutionLanes(_) => "contract_execution_lanes",
        ConfigSettingEntry::EvictionIterator(_) => "eviction_iterator",
        _ => "unknown",
    };
    json!({ "config_setting_id": id })
}

// ---------------------------------------------------------------------------
// TTL
// ---------------------------------------------------------------------------

fn ttl_key(t: &TtlEntry) -> Value {
    json!({ "key_hash": hex::encode(t.key_hash.0) })
}

fn ttl_data(t: &TtlEntry) -> Value {
    json!({
        "key_hash": hex::encode(t.key_hash.0),
        "live_until_ledger_seq": t.live_until_ledger_seq,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_account_entry(account_id: AccountId, balance: i64) -> LedgerEntry {
        LedgerEntry {
            last_modified_ledger_seq: 100,
            data: LedgerEntryData::Account(AccountEntry {
                account_id,
                balance,
                seq_num: SequenceNumber(1),
                num_sub_entries: 0,
                inflation_dest: None,
                flags: 0,
                home_domain: String32::default(),
                thresholds: Thresholds([1, 0, 0, 0]),
                signers: VecM::default(),
                ext: AccountEntryExt::V0,
            }),
            ext: LedgerEntryExt::V0,
        }
    }

    fn make_account_id(byte: u8) -> AccountId {
        AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([byte; 32])))
    }

    #[test]
    fn extract_created_account() {
        let entry = make_account_entry(make_account_id(0xAA), 1_000_000);
        let change = LedgerEntryChange::Created(entry);
        let changes: LedgerEntryChanges = vec![change].try_into().unwrap();

        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: vec![OperationMeta { changes }].try_into().unwrap(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 1);

        let r = &results[0];
        assert_eq!(r.change_type, "created");
        assert_eq!(r.entry_type, "account");
        assert_eq!(r.operation_index, Some(0));
        assert_eq!(r.change_index, 0);
        assert!(r.data.is_some());
        assert_eq!(r.data.as_ref().unwrap()["balance"], 1_000_000);
        assert!(r.key["account_id"].as_str().unwrap().starts_with('G'));
    }

    #[test]
    fn extract_removed_account() {
        let account_id = make_account_id(0xBB);
        let change = LedgerEntryChange::Removed(LedgerKey::Account(LedgerKeyAccount {
            account_id: account_id.clone(),
        }));
        let changes: LedgerEntryChanges = vec![change].try_into().unwrap();

        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: changes,
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 1);

        let r = &results[0];
        assert_eq!(r.change_type, "removed");
        assert_eq!(r.entry_type, "account");
        assert!(r.data.is_none());
        assert!(r.operation_index.is_none());
    }

    #[test]
    fn state_and_updated_pair() {
        let account_id = make_account_id(0xCC);
        let state_entry = make_account_entry(account_id.clone(), 500);
        let updated_entry = make_account_entry(account_id, 1000);

        let changes: LedgerEntryChanges = vec![
            LedgerEntryChange::State(state_entry),
            LedgerEntryChange::Updated(updated_entry),
        ]
        .try_into()
        .unwrap();

        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: vec![OperationMeta { changes }].try_into().unwrap(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].change_type, "state");
        assert_eq!(results[0].data.as_ref().unwrap()["balance"], 500);
        assert_eq!(results[1].change_type, "updated");
        assert_eq!(results[1].data.as_ref().unwrap()["balance"], 1000);
    }

    #[test]
    fn tx_changes_before_and_after_ordering() {
        let before_entry = make_account_entry(make_account_id(0x01), 100);
        let after_entry = make_account_entry(make_account_id(0x02), 200);
        let op_entry = make_account_entry(make_account_id(0x03), 300);

        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: vec![LedgerEntryChange::Created(before_entry)]
                .try_into()
                .unwrap(),
            operations: vec![OperationMeta {
                changes: vec![LedgerEntryChange::Created(op_entry)]
                    .try_into()
                    .unwrap(),
            }]
            .try_into()
            .unwrap(),
            tx_changes_after: vec![LedgerEntryChange::Updated(after_entry)]
                .try_into()
                .unwrap(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 3);

        // tx_changes_before
        assert_eq!(results[0].change_index, 0);
        assert!(results[0].operation_index.is_none());
        assert_eq!(results[0].data.as_ref().unwrap()["balance"], 100);

        // operation changes
        assert_eq!(results[1].change_index, 1);
        assert_eq!(results[1].operation_index, Some(0));
        assert_eq!(results[1].data.as_ref().unwrap()["balance"], 300);

        // tx_changes_after
        assert_eq!(results[2].change_index, 2);
        assert!(results[2].operation_index.is_none());
        assert_eq!(results[2].data.as_ref().unwrap()["balance"], 200);
    }

    #[test]
    fn extract_trustline_change() {
        let asset = TrustLineAsset::CreditAlphanum4(AlphaNum4 {
            asset_code: AssetCode4(*b"USDC"),
            issuer: make_account_id(0xDD),
        });
        let entry = LedgerEntry {
            last_modified_ledger_seq: 100,
            data: LedgerEntryData::Trustline(TrustLineEntry {
                account_id: make_account_id(0xAA),
                asset,
                balance: 5000,
                limit: 10000,
                flags: 1,
                ext: TrustLineEntryExt::V0,
            }),
            ext: LedgerEntryExt::V0,
        };

        let changes: LedgerEntryChanges =
            vec![LedgerEntryChange::Created(entry)].try_into().unwrap();
        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: changes,
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry_type, "trustline");
        assert_eq!(results[0].data.as_ref().unwrap()["balance"], 5000);
        assert_eq!(results[0].key["asset"]["code"], "USDC");
    }

    #[test]
    fn extract_offer_change() {
        let entry = LedgerEntry {
            last_modified_ledger_seq: 100,
            data: LedgerEntryData::Offer(OfferEntry {
                seller_id: make_account_id(0xAA),
                offer_id: 42,
                selling: Asset::Native,
                buying: Asset::Native,
                amount: 1000,
                price: Price { n: 1, d: 2 },
                flags: 0,
                ext: OfferEntryExt::V0,
            }),
            ext: LedgerEntryExt::V0,
        };

        let changes: LedgerEntryChanges =
            vec![LedgerEntryChange::Created(entry)].try_into().unwrap();
        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: changes,
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry_type, "offer");
        assert_eq!(results[0].key["offer_id"], 42);
        assert_eq!(results[0].data.as_ref().unwrap()["amount"], 1000);
    }

    #[test]
    fn extract_contract_data_change() {
        let contract = ScAddress::Contract(ContractId(Hash([0xCC; 32])));
        let entry = LedgerEntry {
            last_modified_ledger_seq: 100,
            data: LedgerEntryData::ContractData(ContractDataEntry {
                ext: ExtensionPoint::V0,
                contract: contract.clone(),
                key: ScVal::Symbol(ScSymbol::try_from("counter".as_bytes().to_vec()).unwrap()),
                durability: ContractDataDurability::Persistent,
                val: ScVal::U64(99),
            }),
            ext: LedgerEntryExt::V0,
        };

        let changes: LedgerEntryChanges =
            vec![LedgerEntryChange::Created(entry)].try_into().unwrap();
        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: changes,
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry_type, "contract_data");
        assert_eq!(results[0].key["durability"], "persistent");
        assert_eq!(results[0].data.as_ref().unwrap()["val"]["type"], "u64");
        assert_eq!(results[0].data.as_ref().unwrap()["val"]["value"], 99);
    }

    #[test]
    fn extract_contract_code_change() {
        let entry = LedgerEntry {
            last_modified_ledger_seq: 100,
            data: LedgerEntryData::ContractCode(ContractCodeEntry {
                ext: ContractCodeEntryExt::V0,
                hash: Hash([0xEE; 32]),
                code: vec![0x00, 0x61, 0x73, 0x6d].try_into().unwrap(),
            }),
            ext: LedgerEntryExt::V0,
        };

        let changes: LedgerEntryChanges =
            vec![LedgerEntryChange::Created(entry)].try_into().unwrap();
        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: changes,
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry_type, "contract_code");
        assert_eq!(results[0].key["hash"], "ee".repeat(32));
        assert_eq!(results[0].data.as_ref().unwrap()["code_byte_len"], 4);
    }

    #[test]
    fn extract_ttl_change() {
        let entry = LedgerEntry {
            last_modified_ledger_seq: 100,
            data: LedgerEntryData::Ttl(TtlEntry {
                key_hash: Hash([0xFF; 32]),
                live_until_ledger_seq: 5000,
            }),
            ext: LedgerEntryExt::V0,
        };

        let changes: LedgerEntryChanges =
            vec![LedgerEntryChange::Updated(entry)].try_into().unwrap();
        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: changes,
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry_type, "ttl");
        assert_eq!(results[0].change_type, "updated");
        assert_eq!(
            results[0].data.as_ref().unwrap()["live_until_ledger_seq"],
            5000
        );
    }

    #[test]
    fn no_changes_for_non_v3v4() {
        let tx_meta = TransactionMeta::V0(VecM::default());
        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert!(results.is_empty());
    }

    #[test]
    fn v4_meta_extraction() {
        let entry = make_account_entry(make_account_id(0xAA), 500);
        let changes: LedgerEntryChanges =
            vec![LedgerEntryChange::Created(entry)].try_into().unwrap();

        let tx_meta = TransactionMeta::V4(TransactionMetaV4 {
            ext: ExtensionPoint::V0,
            tx_changes_before: changes,
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
            events: VecM::default(),
            diagnostic_events: VecM::default(),
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry_type, "account");
        assert_eq!(results[0].data.as_ref().unwrap()["balance"], 500);
    }

    #[test]
    fn multiple_operations_track_index() {
        let entry1 = make_account_entry(make_account_id(0x01), 100);
        let entry2 = make_account_entry(make_account_id(0x02), 200);

        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: vec![
                OperationMeta {
                    changes: vec![LedgerEntryChange::Created(entry1)].try_into().unwrap(),
                },
                OperationMeta {
                    changes: vec![LedgerEntryChange::Created(entry2)].try_into().unwrap(),
                },
            ]
            .try_into()
            .unwrap(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].operation_index, Some(0));
        assert_eq!(results[0].change_index, 0);
        assert_eq!(results[1].operation_index, Some(1));
        assert_eq!(results[1].change_index, 1);
    }

    #[test]
    fn extract_liquidity_pool_change() {
        let entry = LedgerEntry {
            last_modified_ledger_seq: 100,
            data: LedgerEntryData::LiquidityPool(LiquidityPoolEntry {
                liquidity_pool_id: PoolId(Hash([0xAB; 32])),
                body: LiquidityPoolEntryBody::LiquidityPoolConstantProduct(
                    LiquidityPoolEntryConstantProduct {
                        params: LiquidityPoolConstantProductParameters {
                            asset_a: Asset::Native,
                            asset_b: Asset::Native,
                            fee: 30,
                        },
                        reserve_a: 10000,
                        reserve_b: 20000,
                        total_pool_shares: 5000,
                        pool_shares_trust_line_count: 3,
                    },
                ),
            }),
            ext: LedgerEntryExt::V0,
        };

        let changes: LedgerEntryChanges =
            vec![LedgerEntryChange::Created(entry)].try_into().unwrap();
        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: changes,
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry_type, "liquidity_pool");
        assert_eq!(results[0].data.as_ref().unwrap()["reserve_a"], 10000);
        assert_eq!(results[0].data.as_ref().unwrap()["reserve_b"], 20000);
        assert_eq!(results[0].data.as_ref().unwrap()["params"]["fee"], 30);
    }

    #[test]
    fn removed_contract_data_key_only() {
        let contract = ScAddress::Contract(ContractId(Hash([0xCC; 32])));
        let change = LedgerEntryChange::Removed(LedgerKey::ContractData(LedgerKeyContractData {
            contract: contract.clone(),
            key: ScVal::Symbol(ScSymbol::try_from("counter".as_bytes().to_vec()).unwrap()),
            durability: ContractDataDurability::Temporary,
        }));

        let changes: LedgerEntryChanges = vec![change].try_into().unwrap();
        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: changes,
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let results = extract_ledger_entry_changes(&tx_meta, "abc123", 100, 1700000000);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].change_type, "removed");
        assert_eq!(results[0].entry_type, "contract_data");
        assert!(results[0].data.is_none());
        assert_eq!(results[0].key["durability"], "temporary");
        assert!(
            results[0].key["contract"]
                .as_str()
                .unwrap()
                .starts_with('C')
        );
    }
}
