//! Value movement per (transaction, asset), derived from LEDGER balance-entry
//! changes — the authoritative, consensus-verified source (a contract cannot forge
//! a ledger balance). Token EVENTS are logs and are NEVER used for value (task
//! 0393 redesign; see 0415). Covers EVERY tx, classic or Soroban.
//!
//! The balance carriers:
//! - `AccountEntry.balance` (native XLM),
//! - `TrustLineEntry.balance` (classic issued assets),
//! - `ContractData` `Balance(Address)` — a Soroban token balance held by an
//!   account or contract: a SAC `BalanceValue` **struct** (a classic/native asset
//!   held by a contract) or a **bare `i128`** (a bespoke token, which IS the asset),
//! - `LiquidityPoolEntry` reserves — the pool (`L…`) as holder of its two
//!   reserve assets (task 0540 / T03; before it, value routed *through* a classic
//!   pool netted to zero for the only holder the reader saw — 7.6% of
//!   value-moving transactions),
//! - `ClaimableBalanceEntry.amount` — the balance (`B…`) as holder of its asset
//!   between creation and claim.
//!
//! Pool-share trustlines stay unread on purpose: CAP-67 emits no token event for
//! pool shares (measured: not one `mint`/`burn` labelled with a pool share on
//! 60 000 ledgers — a deposit is two `transfer`s to the `L…` address), so a
//! share balance has no event-side counterpart to reconcile against.
//!
//! **Every** value flow — payment, path payment, offer/DEX fill, LP deposit/
//! withdraw, claimable-balance create/claim, clawback, and Soroban SAC/bespoke
//! transfers — settles as a change to one or more of these, so a single
//! before→after delta reader over `TransactionMeta` covers all of them uniformly
//! and auto-nets routing hops (a pass-through holder ends at delta 0).
//!
//! ## Fee
//!
//! The transaction fee is charged in the ledger's separate `feeProcessing`
//! phase, **not** in `TransactionMeta` (the apply phase), so these deltas never
//! include the charge. The Soroban unused-resource-fee REFUND is different:
//! before Protocol 23 it lands in `tx_changes_after` (from 23 on it is outside
//! `TransactionMeta`, in `post_tx_apply_fee_processing`). [`ledger_balance_deltas`]
//! includes it; [`operation_balance_deltas`] does not — use the latter when the
//! question is "what did the operations move". Found by the task 0540 oracle,
//! not by reading the spec. (A seq-number bump on the source appears in
//! `tx_changes_before`, but it does not move `balance`, so it nets to a 0 delta
//! and is dropped.)
//!
//! Output is per-(account, asset) signed deltas; the caller resolves the asset
//! surrogate and reduces to `max(Σ+, Σ−)` per asset via `xdr_parser::net_settled`.

use std::collections::BTreeMap;

use stellar_xdr::{
    Asset, ContractDataEntry, LedgerEntry, LedgerEntryChange, LedgerEntryData, LedgerKey,
    LiquidityPoolEntryBody, ScAddress, ScVal, TransactionMeta, TrustLineAsset,
};

use crate::meta::{ledger_changes, operation_changes};

/// The asset a LEDGER balance change moved — the LEDGER domain's asset vocabulary
/// (cf. `AssetRef` for op-declared assets, `EventAsset` for event-named; each domain
/// owns its own small asset enum). Resolved to a DB surrogate by the persistence
/// layer (`ids` / `sac_classic`). `Ord` so it can key the telescoping `BTreeMap`.
///
/// The two `ContractData` cases are **different DB asset types** — a SAC-wrapped
/// classic (type 1) vs a bespoke token (type 3) — so they are two variants, not one
/// variant with a boolean: each resolves a different way.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum LedgerAsset {
    /// Native XLM — from `AccountEntry`.
    Native,
    /// A classic issued asset (code + issuer `G…` StrKey) — from `TrustLineEntry`.
    Credit { code: String, issuer: String },
    /// A classic/native asset held via its Stellar-Asset-Contract **wrapper** — from
    /// a `ContractData` `Balance` whose value is a SAC `BalanceValue` struct. The
    /// `C…` is the SAC contract; the caller reverses it to the wrapped classic
    /// `asset_id` via the registry (`sac_classic`), or drops if unknown. A **classic**
    /// asset (DB type 1), NOT a contract asset.
    SacWrapped(String),
    /// A **bespoke** Soroban token — from a `ContractData` `Balance` whose value is a
    /// bare `i128`. The `C…` token contract IS the asset (DB type 3); resolved to its
    /// own surrogate (`ids::contract_id`).
    Bespoke(String),
}

/// A net balance change for one account on one asset within a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerDelta {
    /// Account G-StrKey.
    pub account: String,
    /// The moved asset. `ledger_balance_deltas` yields `Native` / `Credit` (from
    /// `AccountEntry` / `TrustLineEntry` changes) and `SacWrapped` / `Bespoke` (from
    /// `ContractData` balance changes); the caller resolves each to a surrogate.
    /// Never `Contract` — that identity comes only from an event, and the ledger
    /// reader does not read events.
    pub asset: LedgerAsset,
    /// Signed raw stroops the account's balance moved across the transaction.
    pub delta: i128,
}

/// Per-(account, asset) net classic balance delta for a transaction. Only
/// non-zero deltas are returned, ordered by (account, asset) for determinism.
pub fn ledger_balance_deltas(meta: &TransactionMeta) -> Vec<LedgerDelta> {
    balance_deltas_over(ledger_changes(meta))
}

/// Like [`ledger_balance_deltas`] but over the OPERATIONS' changes only —
/// `tx_changes_after` excluded. That is where a Soroban transaction's fee
/// refund lands before Protocol 23, and a refund is a fee, not a movement:
/// the events-vs-ledger oracle (task 0540 T04) compares token movements, so it
/// reads this view. From Protocol 23 the refund is outside `TransactionMeta`
/// anyway, so the two functions agree there.
pub fn operation_balance_deltas(meta: &TransactionMeta) -> Vec<LedgerDelta> {
    balance_deltas_over(operation_changes(meta))
}

fn balance_deltas_over(changes: Vec<&LedgerEntryChange>) -> Vec<LedgerDelta> {
    // Runs for EVERY tx (classic AND Soroban): the ledger is the authoritative,
    // unspoofable source of value for native/classic/SAC, whether moved by a
    // classic op (Account/Trustline changes) or a Soroban invocation (which also
    // produces contract-held SAC balance changes in `ContractData`). Only bespoke
    // tokens — with no ledger-readable balance — are valued from their events.
    // (account, asset) -> before/after balance, telescoped across the tx.
    let mut acc: BTreeMap<(String, LedgerAsset), Balances> = BTreeMap::new();
    for change in changes {
        match change {
            // `State` / `Restored` are before-images (Restored re-appears from
            // state archival, protocol 23 — the restore itself moves no value).
            LedgerEntryChange::State(e) | LedgerEntryChange::Restored(e) => {
                record(&mut acc, e, false, false);
            }
            LedgerEntryChange::Created(e) => record(&mut acc, e, true, false),
            LedgerEntryChange::Updated(e) => record(&mut acc, e, false, true),
            LedgerEntryChange::Removed(k) => record_removed(&mut acc, k),
        }
    }
    acc.into_iter()
        .filter_map(|((account, asset), b)| {
            // A bespoke token's `ContractData` balance is attacker-authored i128,
            // so this subtraction can overflow; release builds run with
            // `overflow-checks = false`, so an unchecked `-` would wrap into a
            // fabricated figure instead of panicking. `checked_sub` → `?` drops
            // the delta (no value) on overflow rather than lying. (Native/classic
            // balances are i64-wide and cannot overflow here.)
            let delta = b.last.checked_sub(b.initial.unwrap_or(0))?;
            (delta != 0).then_some(LedgerDelta {
                account,
                asset,
                delta,
            })
        })
        .collect()
}

/// Running (initial, last) balance for one (account, asset) key.
struct Balances {
    initial: Option<i128>,
    last: i128,
}

/// Fold a balance-bearing entry into the running map. `created` marks a
/// `Created` change (the entry did not exist before → initial balance 0);
/// otherwise the first change seen (a `State` before-image, or an `Updated`
/// with no preceding state) sets the initial balance.
fn record(
    acc: &mut BTreeMap<(String, LedgerAsset), Balances>,
    entry: &LedgerEntry,
    created: bool,
    is_update: bool,
) {
    for (account, asset, balance) in entry_balances(entry) {
        record_one(acc, account, asset, balance, created, is_update);
    }
}

fn record_one(
    acc: &mut BTreeMap<(String, LedgerAsset), Balances>,
    account: String,
    asset: LedgerAsset,
    balance: i128,
    created: bool,
    is_update: bool,
) {
    let e = acc.entry((account, asset)).or_insert(Balances {
        initial: None,
        last: 0,
    });
    if e.initial.is_none() {
        // Telescoping invariant (#11): the FIRST change for a key must be a
        // before-image (`State`/`Restored`) or a `Created` — never an `Updated`.
        // If an `Updated` were first, `balance` is the AFTER value and using it as
        // `initial` silently zeroes that step's delta. Stellar always emits a
        // `State` before an `Updated`, so this is debug-only (zero release cost);
        // it fails a test/dev build fast if a malformed or future-protocol meta
        // ever violates it, instead of understating in silence.
        debug_assert!(
            !is_update,
            "classic telescoping: Updated with no preceding State/Created — pre-image lost, delta understated"
        );
        e.initial = Some(if created { 0 } else { balance });
    }
    e.last = balance;
}

/// A removed entry drops to balance 0 (its initial came from a preceding State).
fn record_removed(acc: &mut BTreeMap<(String, LedgerAsset), Balances>, key: &LedgerKey) {
    // A pool or claimable-balance key names the holder but not its asset(s);
    // those came from the preceding `State`, so every balance of that holder
    // drops to 0. (Stellar removes a pool only when its reserves are already 0
    // and a claimable balance only when claimed, so the drop IS the movement.)
    let holder = match key {
        LedgerKey::LiquidityPool(k) => {
            Some(ScAddress::LiquidityPool(k.liquidity_pool_id.clone()).to_string())
        }
        LedgerKey::ClaimableBalance(k) => {
            Some(ScAddress::ClaimableBalance(k.balance_id.clone()).to_string())
        }
        _ => None,
    };
    if let Some(holder) = holder {
        for ((account, _), b) in acc.iter_mut() {
            if *account == holder {
                b.last = 0;
            }
        }
        return;
    }
    let Some((account, asset)) = removed_balance_key(key) else {
        return;
    };
    acc.entry((account, asset))
        .or_insert(Balances {
            initial: None,
            last: 0,
        })
        .last = 0;
}

/// Every `(holder, asset, balance)` an entry carries: one for `AccountEntry`
/// (native), `TrustLineEntry` (classic credit), a `ContractData`
/// `Balance(Address)` (SAC struct or bespoke i128) and `ClaimableBalanceEntry`
/// (the `B…` balance as holder); **two** for a `LiquidityPoolEntry` (the `L…`
/// pool as holder of each reserve). Empty for everything else (offers, data
/// entries, pool-share trustlines, non-balance ContractData) — an offer's
/// effect surfaces as the maker's trustline change when it is crossed.
///
/// Holder StrKeys use the same rendering as CAP-67 event topics
/// (`ScAddress::{LiquidityPool, ClaimableBalance}` → `L…` / `B…`), so the
/// events-vs-ledger oracle (task 0540 T04) compares like with like.
fn entry_balances(entry: &LedgerEntry) -> Vec<(String, LedgerAsset, i128)> {
    match &entry.data {
        LedgerEntryData::Account(a) => vec![(
            a.account_id.to_string(),
            LedgerAsset::Native,
            i128::from(a.balance),
        )],
        LedgerEntryData::Trustline(t) => trustline_event_asset(&t.asset)
            .map(|asset| (t.account_id.to_string(), asset, i128::from(t.balance)))
            .into_iter()
            .collect(),
        LedgerEntryData::ContractData(cd) => contract_data_balance(cd).into_iter().collect(),
        LedgerEntryData::LiquidityPool(lp) => {
            let holder = ScAddress::LiquidityPool(lp.liquidity_pool_id.clone()).to_string();
            let LiquidityPoolEntryBody::LiquidityPoolConstantProduct(cp) = &lp.body;
            vec![
                (
                    holder.clone(),
                    classic_asset(&cp.params.asset_a),
                    i128::from(cp.reserve_a),
                ),
                (
                    holder,
                    classic_asset(&cp.params.asset_b),
                    i128::from(cp.reserve_b),
                ),
            ]
        }
        LedgerEntryData::ClaimableBalance(cb) => vec![(
            ScAddress::ClaimableBalance(cb.balance_id.clone()).to_string(),
            classic_asset(&cb.asset),
            i128::from(cb.amount),
        )],
        _ => Vec::new(),
    }
}

/// The `LedgerAsset` of a classic `Asset` (pool reserves, claimable balances).
fn classic_asset(asset: &Asset) -> LedgerAsset {
    match asset {
        Asset::Native => LedgerAsset::Native,
        Asset::CreditAlphanum4(a) => LedgerAsset::Credit {
            code: crate::asset_code::asset_code_str(a.asset_code.as_slice()),
            issuer: a.issuer.to_string(),
        },
        Asset::CreditAlphanum12(a) => LedgerAsset::Credit {
            code: crate::asset_code::asset_code_str(a.asset_code.as_slice()),
            issuer: a.issuer.to_string(),
        },
    }
}

/// `(account, asset)` for a removed balance key; `None` otherwise. Covers a
/// removed account/trustline and a removed SAC contract-balance entry
/// (`ContractData` `Balance(Address)` dropping to 0).
fn removed_balance_key(key: &LedgerKey) -> Option<(String, LedgerAsset)> {
    match key {
        LedgerKey::Account(k) => Some((k.account_id.to_string(), LedgerAsset::Native)),
        LedgerKey::Trustline(k) => {
            Some((k.account_id.to_string(), trustline_event_asset(&k.asset)?))
        }
        // Removed carries no value, so it cannot tell a SAC balance (struct) from a
        // bespoke one (bare i128) — the distinction lives in the value it dropped.
        // A balance normally hits 0 via `Updated`, not `Removed`, so this rare edge
        // is left unhandled rather than risk keying the wrong asset variant.
        _ => None,
    }
}

/// A Soroban token balance change: a `ContractData` entry with key
/// `Balance(Address)`. Two value shapes, both authoritative (the actual stored
/// balance, unspoofable):
/// - a SAC `BalanceValue` **struct** (`Map{ amount, authorized, clawback }`) — a
///   contract holding a classic/native asset → `SacWrapped(sac StrKey)` (the caller
///   re-maps it to the wrapped classic asset).
/// - a **bare `i128`** — a bespoke Soroban token's own balance →
///   `Bespoke(token StrKey)` (the token IS the asset).
///
/// `None` for any other `ContractData` (TTL bumps, non-`Balance` keys, unknown
/// value shapes). Verified against a real mainnet Soroban SAC transfer
/// (contract↔contract): `key` = `Vec[Symbol("Balance"), Address(holder)]`, `val` =
/// the struct, emitted `State`(before) + `Updated`(after) so it telescopes.
fn contract_data_balance(cd: &ContractDataEntry) -> Option<(String, LedgerAsset, i128)> {
    let holder = balance_key_holder(&cd.key)?;
    let contract = cd.contract.to_string();
    if let Some(amount) = sac_balance_struct_amount(&cd.val) {
        // Struct value → a classic/native asset held via its SAC wrapper.
        return Some((holder, LedgerAsset::SacWrapped(contract), amount));
    }
    if let ScVal::I128(p) = &cd.val {
        // Bare i128 → a bespoke token; the contract IS the asset.
        let amount = (i128::from(p.hi) << 64) | i128::from(p.lo);
        return Some((holder, LedgerAsset::Bespoke(contract), amount));
    }
    None
}

/// The holder StrKey from a `Balance(Address)` `ContractData` key
/// (`Vec[Symbol("Balance"), Address(holder)]`); `None` for any other key shape.
fn balance_key_holder(key: &ScVal) -> Option<String> {
    let ScVal::Vec(Some(v)) = key else {
        return None;
    };
    let [tag, holder] = v.as_slice() else {
        return None;
    };
    match tag {
        ScVal::Symbol(s) if s.to_string() == "Balance" => {}
        _ => return None,
    }
    match holder {
        ScVal::Address(addr) => Some(addr.to_string()),
        _ => None,
    }
}

/// The `amount` field of a SAC `BalanceValue` struct (`ScVal::Map`); `None` for a
/// bare i128 (bespoke) or any other value shape.
fn sac_balance_struct_amount(val: &ScVal) -> Option<i128> {
    let ScVal::Map(Some(m)) = val else {
        return None;
    };
    for entry in m.iter() {
        if let ScVal::Symbol(k) = &entry.key
            && k.to_string() == "amount"
            && let ScVal::I128(p) = &entry.val
        {
            return Some((i128::from(p.hi) << 64) | i128::from(p.lo));
        }
    }
    None
}

/// The `LedgerAsset` identity of a trustline asset; `None` for pool shares (not a
/// single-asset balance). `Native` is included for completeness though native
/// balances live on `AccountEntry`, not a trustline.
fn trustline_event_asset(asset: &TrustLineAsset) -> Option<LedgerAsset> {
    match asset {
        TrustLineAsset::Native => Some(LedgerAsset::Native),
        TrustLineAsset::CreditAlphanum4(a) => Some(LedgerAsset::Credit {
            code: crate::asset_code::asset_code_str(a.asset_code.as_slice()),
            issuer: a.issuer.to_string(),
        }),
        TrustLineAsset::CreditAlphanum12(a) => Some(LedgerAsset::Credit {
            code: crate::asset_code::asset_code_str(a.asset_code.as_slice()),
            issuer: a.issuer.to_string(),
        }),
        TrustLineAsset::PoolShare(_) => None,
    }
}

#[cfg(test)]
#[path = "ledger_value_tests.rs"]
mod tests;
