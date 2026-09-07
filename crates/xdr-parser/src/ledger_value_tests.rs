use super::*;
use stellar_xdr::{
    AccountEntry, AccountEntryExt, AccountId, AlphaNum4, AssetCode4, ExtensionPoint, Hash,
    LedgerEntryChanges, LedgerEntryExt, LedgerKeyTrustLine, OperationMeta, PoolId, PublicKey,
    SequenceNumber, String32, Thresholds, TransactionMetaV3, TrustLineEntry, TrustLineEntryExt,
    Uint256, VecM,
};

fn acct_id(b: u8) -> AccountId {
    AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([b; 32])))
}
fn strkey(b: u8) -> String {
    acct_id(b).to_string()
}

fn account_entry(id: u8, balance: i64) -> LedgerEntry {
    LedgerEntry {
        last_modified_ledger_seq: 100,
        data: LedgerEntryData::Account(AccountEntry {
            account_id: acct_id(id),
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

fn usdc_asset(issuer: u8) -> TrustLineAsset {
    TrustLineAsset::CreditAlphanum4(AlphaNum4 {
        asset_code: AssetCode4(*b"USDC"),
        issuer: acct_id(issuer),
    })
}

fn trustline_entry(holder: u8, asset: TrustLineAsset, balance: i64) -> LedgerEntry {
    LedgerEntry {
        last_modified_ledger_seq: 100,
        data: LedgerEntryData::Trustline(TrustLineEntry {
            account_id: acct_id(holder),
            asset,
            balance,
            limit: i64::MAX,
            flags: 1,
            ext: TrustLineEntryExt::V0,
        }),
        ext: LedgerEntryExt::V0,
    }
}

/// Build a V3 meta whose single operation carries `changes`.
fn meta_with_op_changes(changes: Vec<LedgerEntryChange>) -> TransactionMeta {
    TransactionMeta::V3(TransactionMetaV3 {
        ext: ExtensionPoint::V0,
        tx_changes_before: LedgerEntryChanges::default(),
        operations: vec![OperationMeta {
            changes: changes.try_into().unwrap(),
        }]
        .try_into()
        .unwrap(),
        tx_changes_after: LedgerEntryChanges::default(),
        soroban_meta: None,
    })
}

fn find<'a>(d: &'a [LedgerDelta], account: &str, asset: &LedgerAsset) -> Option<&'a LedgerDelta> {
    d.iter().find(|x| x.account == account && &x.asset == asset)
}

fn usdc_credit() -> LedgerAsset {
    LedgerAsset::Credit {
        code: "USDC".to_string(),
        issuer: strkey(0x11),
    }
}

#[test]
fn native_payment_nets_source_and_destination() {
    // source 1000 -> 900, dest 500 -> 600 (payment of 100 native).
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(account_entry(0xAA, 1000)),
        LedgerEntryChange::Updated(account_entry(0xAA, 900)),
        LedgerEntryChange::State(account_entry(0xBB, 500)),
        LedgerEntryChange::Updated(account_entry(0xBB, 600)),
    ]);
    let d = ledger_balance_deltas(&meta);
    assert_eq!(
        find(&d, &strkey(0xAA), &LedgerAsset::Native).unwrap().delta,
        -100
    );
    assert_eq!(
        find(&d, &strkey(0xBB), &LedgerAsset::Native).unwrap().delta,
        100
    );
}

#[test]
fn created_account_funding_moves_native() {
    // funder 1000 -> 700, new account created at 300.
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(account_entry(0xAA, 1000)),
        LedgerEntryChange::Updated(account_entry(0xAA, 700)),
        LedgerEntryChange::Created(account_entry(0xCC, 300)),
    ]);
    let d = ledger_balance_deltas(&meta);
    assert_eq!(
        find(&d, &strkey(0xAA), &LedgerAsset::Native).unwrap().delta,
        -300
    );
    assert_eq!(
        find(&d, &strkey(0xCC), &LedgerAsset::Native).unwrap().delta,
        300
    );
}

#[test]
fn credit_trustline_payment_uses_credit_asset() {
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(trustline_entry(0xAA, usdc_asset(0x11), 1000)),
        LedgerEntryChange::Updated(trustline_entry(0xAA, usdc_asset(0x11), 850)),
    ]);
    let d = ledger_balance_deltas(&meta);
    assert_eq!(find(&d, &strkey(0xAA), &usdc_credit()).unwrap().delta, -150);
}

#[test]
fn removed_trustline_zeroes_the_balance() {
    // trustline 100 -> removed: delta -100.
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(trustline_entry(0xAA, usdc_asset(0x11), 100)),
        LedgerEntryChange::Removed(LedgerKey::Trustline(LedgerKeyTrustLine {
            account_id: acct_id(0xAA),
            asset: usdc_asset(0x11),
        })),
    ]);
    let d = ledger_balance_deltas(&meta);
    assert_eq!(find(&d, &strkey(0xAA), &usdc_credit()).unwrap().delta, -100);
}

#[test]
fn balance_unchanged_update_is_dropped() {
    // seq-number bump only: balance 1000 -> 1000, delta 0, no row.
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(account_entry(0xAA, 1000)),
        LedgerEntryChange::Updated(account_entry(0xAA, 1000)),
    ]);
    assert!(ledger_balance_deltas(&meta).is_empty());
}

#[test]
fn only_the_first_image_sets_initial_across_repeated_changes() {
    // Two ops touch the same account, each emitting its own State/Updated
    // pair: 1000 -> 900, then 900 -> 850. Expected: ONE row, delta -150,
    // measured first image to last.
    //
    // This pins the `if initial.is_none()` guard specifically. Drop it and
    // the second State(900) overwrites initial, so the answer collapses to
    // 850 - 850 = 0 and the whole transaction reads as moving nothing. (It
    // does NOT distinguish telescoping from summing per-step deltas —
    // (900-1000)+(850-900) is -150 either way; that is the telescoping
    // identity, not a difference worth testing.)
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(account_entry(0xAA, 1000)),
        LedgerEntryChange::Updated(account_entry(0xAA, 900)),
        LedgerEntryChange::State(account_entry(0xAA, 900)),
        LedgerEntryChange::Updated(account_entry(0xAA, 850)),
    ]);
    let d = ledger_balance_deltas(&meta);
    assert_eq!(d.len(), 1, "one row per (account, asset), got {d:?}");
    assert_eq!(
        find(&d, &strkey(0xAA), &LedgerAsset::Native).unwrap().delta,
        -150
    );
}

#[test]
fn restored_is_a_before_image_not_a_value_move() {
    // Protocol 23 state archival: the entry re-appears via Restored, which
    // moves no value by itself — it is the "before" for what follows.
    // Restored(1000) alone -> no row; Restored(1000) + Updated(900) -> -100.
    let alone = meta_with_op_changes(vec![LedgerEntryChange::Restored(account_entry(0xAA, 1000))]);
    assert!(
        ledger_balance_deltas(&alone).is_empty(),
        "a bare restore moves nothing"
    );

    let then_spent = meta_with_op_changes(vec![
        LedgerEntryChange::Restored(account_entry(0xAA, 1000)),
        LedgerEntryChange::Updated(account_entry(0xAA, 900)),
    ]);
    let d = ledger_balance_deltas(&then_spent);
    assert_eq!(
        find(&d, &strkey(0xAA), &LedgerAsset::Native).unwrap().delta,
        -100
    );
}

#[test]
fn pool_share_trustline_is_not_a_single_asset_balance() {
    // A pool-share trustline balance is LP shares, not an asset amount —
    // counting it would invent value on every LP deposit/withdraw.
    let pool_share = TrustLineAsset::PoolShare(PoolId(Hash([0x22; 32])));
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(trustline_entry(0xAA, pool_share.clone(), 100)),
        LedgerEntryChange::Updated(trustline_entry(0xAA, pool_share, 250)),
    ]);
    assert!(ledger_balance_deltas(&meta).is_empty());
}

// ---- ContractData Soroban balances (task 0393 ledger redesign) --------
// Synthetic coverage of `contract_data_balance` / `sac_balance_struct_amount`
// / `balance_key_holder`. (A real-mainnet decode also lives in
// `tests/net_settled_ledger_contractdata.rs`, gated on a captured fixture.)

fn contract_addr(b: u8) -> stellar_xdr::ScAddress {
    stellar_xdr::ScAddress::Contract(stellar_xdr::ContractId(Hash([b; 32])))
}

fn i128_scval(v: i128) -> ScVal {
    ScVal::I128(stellar_xdr::Int128Parts {
        hi: (v >> 64) as i64,
        lo: v as u64,
    })
}

/// A SAC `BalanceValue` struct value: `Map{ amount, authorized, clawback }`.
fn sac_balance_struct(amount: i128) -> ScVal {
    use stellar_xdr::{ScMap, ScMapEntry, ScSymbol};
    let entry = |k: &[u8], val: ScVal| ScMapEntry {
        key: ScVal::Symbol(ScSymbol::try_from(k.to_vec()).unwrap()),
        val,
    };
    ScVal::Map(Some(
        ScMap::try_from(vec![
            entry(b"amount", i128_scval(amount)),
            entry(b"authorized", ScVal::Bool(true)),
            entry(b"clawback", ScVal::Bool(false)),
        ])
        .unwrap(),
    ))
}

/// A `ContractData` `Balance(Address)` entry for `holder` under token/SAC
/// contract `token`, carrying `val`.
fn contract_data_balance_entry(token: u8, holder: u8, val: ScVal) -> LedgerEntry {
    use stellar_xdr::{ContractDataDurability, ContractDataEntry, ScSymbol, ScVec};
    let key = ScVal::Vec(Some(
        ScVec::try_from(vec![
            ScVal::Symbol(ScSymbol::try_from(b"Balance".to_vec()).unwrap()),
            ScVal::Address(contract_addr(holder)),
        ])
        .unwrap(),
    ));
    LedgerEntry {
        last_modified_ledger_seq: 100,
        data: LedgerEntryData::ContractData(ContractDataEntry {
            ext: ExtensionPoint::V0,
            contract: contract_addr(token),
            key,
            durability: ContractDataDurability::Persistent,
            val,
        }),
        ext: LedgerEntryExt::V0,
    }
}

#[test]
fn sac_contract_held_balance_telescopes_to_signed_delta() {
    // A contract's SAC balance 1000 -> 250: it SENT 750 → SacWrapped delta -750.
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(contract_data_balance_entry(
            0x0A,
            0x0B,
            sac_balance_struct(1000),
        )),
        LedgerEntryChange::Updated(contract_data_balance_entry(
            0x0A,
            0x0B,
            sac_balance_struct(250),
        )),
    ]);
    let d = ledger_balance_deltas(&meta);
    let sac: Vec<_> = d
        .iter()
        .filter(|x| matches!(x.asset, LedgerAsset::SacWrapped(_)))
        .collect();
    assert_eq!(sac.len(), 1, "one SAC delta, got {d:?}");
    assert_eq!(sac[0].delta, -750);
}

#[test]
fn bespoke_token_bare_i128_balance_telescopes_to_signed_delta() {
    // A bespoke token balance 500 -> 800: it RECEIVED 300 → Bespoke delta +300.
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(contract_data_balance_entry(0x1A, 0x1B, i128_scval(500))),
        LedgerEntryChange::Updated(contract_data_balance_entry(0x1A, 0x1B, i128_scval(800))),
    ]);
    let d = ledger_balance_deltas(&meta);
    let tok: Vec<_> = d
        .iter()
        .filter(|x| matches!(x.asset, LedgerAsset::Bespoke(_)))
        .collect();
    assert_eq!(tok.len(), 1, "one bespoke delta, got {d:?}");
    assert_eq!(tok[0].delta, 300);
}

#[test]
fn contract_data_non_balance_key_is_ignored() {
    // A ContractData entry whose key is not `Balance(Address)` carries no balance.
    use stellar_xdr::{ContractDataDurability, ContractDataEntry, ScSymbol};
    let entry = LedgerEntry {
        last_modified_ledger_seq: 100,
        data: LedgerEntryData::ContractData(ContractDataEntry {
            ext: ExtensionPoint::V0,
            contract: contract_addr(0x2A),
            key: ScVal::Symbol(ScSymbol::try_from(b"Admin".to_vec()).unwrap()),
            durability: ContractDataDurability::Persistent,
            val: i128_scval(999),
        }),
        ext: LedgerEntryExt::V0,
    };
    let meta = meta_with_op_changes(vec![LedgerEntryChange::State(entry)]);
    assert!(ledger_balance_deltas(&meta).is_empty());
}

// ---- task 0540 / T03: pools and claimable balances are holders too ----------

fn pool_id(b: u8) -> PoolId {
    PoolId(Hash([b; 32]))
}

fn pool_entry(id: u8, reserve_a: i64, reserve_b: i64) -> LedgerEntry {
    use stellar_xdr::{
        LiquidityPoolConstantProductParameters, LiquidityPoolEntry, LiquidityPoolEntryBody,
        LiquidityPoolEntryConstantProduct,
    };
    LedgerEntry {
        last_modified_ledger_seq: 100,
        data: LedgerEntryData::LiquidityPool(LiquidityPoolEntry {
            liquidity_pool_id: pool_id(id),
            body: LiquidityPoolEntryBody::LiquidityPoolConstantProduct(
                LiquidityPoolEntryConstantProduct {
                    params: LiquidityPoolConstantProductParameters {
                        asset_a: stellar_xdr::Asset::Native,
                        asset_b: stellar_xdr::Asset::CreditAlphanum4(AlphaNum4 {
                            asset_code: AssetCode4(*b"USDC"),
                            issuer: acct_id(0x11),
                        }),
                        fee: 30,
                    },
                    reserve_a,
                    reserve_b,
                    total_pool_shares: 1_000,
                    pool_shares_trust_line_count: 1,
                },
            ),
        }),
        ext: LedgerEntryExt::V0,
    }
}

fn cb_id(b: u8) -> stellar_xdr::ClaimableBalanceId {
    stellar_xdr::ClaimableBalanceId::ClaimableBalanceIdTypeV0(Hash([b; 32]))
}

fn claimable_balance_entry(id: u8, amount: i64) -> LedgerEntry {
    use stellar_xdr::{ClaimableBalanceEntry, ClaimableBalanceEntryExt};
    LedgerEntry {
        last_modified_ledger_seq: 100,
        data: LedgerEntryData::ClaimableBalance(ClaimableBalanceEntry {
            balance_id: cb_id(id),
            claimants: VecM::default(),
            asset: stellar_xdr::Asset::CreditAlphanum4(AlphaNum4 {
                asset_code: AssetCode4(*b"USDC"),
                issuer: acct_id(0x11),
            }),
            amount,
            ext: ClaimableBalanceEntryExt::V0,
        }),
        ext: LedgerEntryExt::V0,
    }
}

#[test]
fn pool_deposit_credits_the_pool_as_holder_of_both_reserves() {
    let pool = stellar_xdr::ScAddress::LiquidityPool(pool_id(0x77)).to_string();
    assert!(
        pool.starts_with('L'),
        "pool holder renders as the L… StrKey: {pool}"
    );
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(pool_entry(0x77, 1_000, 2_000)),
        LedgerEntryChange::Updated(pool_entry(0x77, 1_050, 2_100)),
        // the depositor's side, so the transaction nets to zero
        LedgerEntryChange::State(account_entry(0x01, 10_000)),
        LedgerEntryChange::Updated(account_entry(0x01, 9_950)),
        LedgerEntryChange::State(trustline_entry(0x01, usdc_asset(0x11), 500)),
        LedgerEntryChange::Updated(trustline_entry(0x01, usdc_asset(0x11), 400)),
    ]);
    let d = ledger_balance_deltas(&meta);
    assert_eq!(find(&d, &pool, &LedgerAsset::Native).unwrap().delta, 50);
    assert_eq!(find(&d, &pool, &usdc_credit()).unwrap().delta, 100);
    assert_eq!(
        find(&d, &strkey(0x01), &LedgerAsset::Native).unwrap().delta,
        -50
    );
    assert_eq!(find(&d, &strkey(0x01), &usdc_credit()).unwrap().delta, -100);
    assert_eq!(
        d.len(),
        4,
        "no pool-share row: shares have no event counterpart"
    );
}

#[test]
fn pool_removal_drops_both_reserves_from_the_preceding_state() {
    let pool = stellar_xdr::ScAddress::LiquidityPool(pool_id(0x77)).to_string();
    let meta = meta_with_op_changes(vec![
        LedgerEntryChange::State(pool_entry(0x77, 30, 40)),
        LedgerEntryChange::Removed(LedgerKey::LiquidityPool(
            stellar_xdr::LedgerKeyLiquidityPool {
                liquidity_pool_id: pool_id(0x77),
            },
        )),
    ]);
    let d = ledger_balance_deltas(&meta);
    assert_eq!(find(&d, &pool, &LedgerAsset::Native).unwrap().delta, -30);
    assert_eq!(find(&d, &pool, &usdc_credit()).unwrap().delta, -40);
}

#[test]
fn claimable_balance_is_a_holder_from_creation_to_claim() {
    let cb = stellar_xdr::ScAddress::ClaimableBalance(cb_id(0x55)).to_string();
    assert!(
        cb.starts_with('B'),
        "claimable balance renders as the B… StrKey: {cb}"
    );

    let created = meta_with_op_changes(vec![
        LedgerEntryChange::Created(claimable_balance_entry(0x55, 700)),
        LedgerEntryChange::State(trustline_entry(0x01, usdc_asset(0x11), 1_000)),
        LedgerEntryChange::Updated(trustline_entry(0x01, usdc_asset(0x11), 300)),
    ]);
    let d = ledger_balance_deltas(&created);
    assert_eq!(find(&d, &cb, &usdc_credit()).unwrap().delta, 700);
    assert_eq!(find(&d, &strkey(0x01), &usdc_credit()).unwrap().delta, -700);

    let claimed = meta_with_op_changes(vec![
        LedgerEntryChange::State(claimable_balance_entry(0x55, 700)),
        LedgerEntryChange::Removed(LedgerKey::ClaimableBalance(
            stellar_xdr::LedgerKeyClaimableBalance {
                balance_id: cb_id(0x55),
            },
        )),
        LedgerEntryChange::State(trustline_entry(0x02, usdc_asset(0x11), 0)),
        LedgerEntryChange::Updated(trustline_entry(0x02, usdc_asset(0x11), 700)),
    ]);
    let d = ledger_balance_deltas(&claimed);
    assert_eq!(find(&d, &cb, &usdc_credit()).unwrap().delta, -700);
    assert_eq!(find(&d, &strkey(0x02), &usdc_credit()).unwrap().delta, 700);
}
