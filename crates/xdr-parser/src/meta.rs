//! `TransactionMeta` ledger-change accessor (task 0359, revived by 0393).
//!
//! Projects the ledger entry changes out of a `TransactionMeta`, whatever its
//! version. Two consumers: [`contract`](crate::contract), which scans them for
//! deployed WASM, and [`ledger_value`](crate::ledger_value), which telescopes
//! the before/after balance images into a transaction's net-settled value.
//!
//! ## Why a module for one function
//!
//! [`ledger_changes`] is exhaustive with **no `_` wildcard**, so a new
//! protocol meta version (e.g. `V5` for Protocol 24) fails to compile HERE
//! rather than being silently absorbed into an empty result. A `_ => empty` arm
//! quietly drops every change of a `V5` transaction while the ledger still
//! commits — the "quiet chain" failure: the index goes blank, no error, all
//! green. Keeping the match in one named place is what makes that a compile
//! error instead of a silent hole.
//!
//! `V0 | V1 | V2` are the pre-Soroban legacy shapes. This is a Soroban-era
//! indexer (backfill floor = ledger 50,457,424, Protocol 20 go-live), so they
//! never occur in the fed range and are an explicit empty arm — not a wildcard.
//!
//! ## Scope — partial adoption, do not oversell it
//!
//! 0359 intended this as THE central meta accessor and wrote six of them; the
//! branch was parked in a stash before adoption. 0393 revived only the accessor
//! it uses. The other five were dropped rather than carried as caller-less `pub`
//! code (which no dead-code lint would ever flag) — they remain in history at
//! commit `ddb021ff` for whoever finishes the adoption.
//!
//! `contract.rs` was migrated onto this function (0393): it had an identical
//! private copy, wildcard arm included. Four modules still match
//! `TransactionMeta` themselves — `ledger_entry_changes` (18 sites), `event`
//! (11), `invocation` (6), `operation` (6), plus `transaction` (1 site, for
//! version detection rather than projection). **`invocation` (×2) and
//! `operation` (×1) still carry `_ =>` wildcard arms** — the exact hole this
//! module exists to close. Migrating them is open work (0393 hygiene notes), so
//! this is not yet the only way to walk meta.
//!
//! ## Adding a new meta version (Protocol 24+)
//!
//! The compiler will point at [`ledger_changes`]. Decide whether the new
//! version carries changes (implement the arm) or not (extend the legacy arm) —
//! never add a `_ =>` wildcard, never stub an empty return.

use stellar_xdr::{LedgerCloseMeta, LedgerEntryChange, LedgerEntryChanges, TransactionMeta};

/// Visit every transaction's apply-time meta of one `LedgerCloseMeta`, in
/// apply order, and return the ledger sequence. Same exhaustive-match
/// philosophy as [`ledger_changes`]: a new close-meta version fails to
/// compile HERE instead of being silently absorbed (review #447 — this
/// V0/V1/V2 unroll was copy-pasted across the raw-ledger test harnesses).
pub fn for_each_tx_meta(
    lcm: &LedgerCloseMeta,
    mut f: impl FnMut(u32, usize, &TransactionMeta),
) -> u32 {
    match lcm {
        LedgerCloseMeta::V0(v0) => {
            let seq = v0.ledger_header.header.ledger_seq;
            for (i, tx) in v0.tx_processing.iter().enumerate() {
                f(seq, i, &tx.tx_apply_processing);
            }
            seq
        }
        LedgerCloseMeta::V1(v1) => {
            let seq = v1.ledger_header.header.ledger_seq;
            for (i, tx) in v1.tx_processing.iter().enumerate() {
                f(seq, i, &tx.tx_apply_processing);
            }
            seq
        }
        LedgerCloseMeta::V2(v2) => {
            let seq = v2.ledger_header.header.ledger_seq;
            for (i, tx) in v2.tx_processing.iter().enumerate() {
                f(seq, i, &tx.tx_apply_processing);
            }
            seq
        }
    }
}

/// Every ledger entry change of a transaction, in canonical order:
/// `tx_changes_before`, then each operation's changes in operation order, then
/// `tx_changes_after`. Empty for legacy meta.
///
/// The order is the contract: a consumer telescoping before→after images relies
/// on seeing a `State` before the `Updated` that follows it.
pub fn ledger_changes(meta: &TransactionMeta) -> Vec<&LedgerEntryChange> {
    match meta {
        TransactionMeta::V3(v3) => collect(
            &v3.tx_changes_before,
            v3.operations.iter().map(|o| &o.changes),
            &v3.tx_changes_after,
        ),
        TransactionMeta::V4(v4) => collect(
            &v4.tx_changes_before,
            v4.operations.iter().map(|o| &o.changes),
            &v4.tx_changes_after,
        ),
        TransactionMeta::V0(_) | TransactionMeta::V1(_) | TransactionMeta::V2(_) => Vec::new(),
    }
}

fn collect<'a>(
    before: &'a LedgerEntryChanges,
    op_changes: impl Iterator<Item = &'a LedgerEntryChanges>,
    after: &'a LedgerEntryChanges,
) -> Vec<&'a LedgerEntryChange> {
    before
        .iter()
        .chain(op_changes.flat_map(|changes| changes.iter()))
        .chain(after.iter())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::{
        AccountId, ExtensionPoint, LedgerKey, LedgerKeyAccount, OperationMeta, PublicKey,
        TransactionMetaV3, Uint256, VecM,
    };

    fn acct(b: u8) -> AccountId {
        AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([b; 32])))
    }

    /// A cheap `Removed` change (no full entry needed) tagged by a byte.
    fn removed(b: u8) -> LedgerEntryChange {
        LedgerEntryChange::Removed(LedgerKey::Account(LedgerKeyAccount {
            account_id: acct(b),
        }))
    }

    /// Recover the byte a [`removed`] change was tagged with.
    fn tag(change: &LedgerEntryChange) -> u8 {
        match change {
            LedgerEntryChange::Removed(LedgerKey::Account(k)) => {
                let PublicKey::PublicKeyTypeEd25519(Uint256(bytes)) = &k.account_id.0;
                bytes[0]
            }
            other => panic!("fixture is always a removed account key, got {other:?}"),
        }
    }

    fn changes(cs: Vec<LedgerEntryChange>) -> LedgerEntryChanges {
        cs.try_into().unwrap()
    }

    fn v3(
        before: Vec<LedgerEntryChange>,
        ops: Vec<Vec<LedgerEntryChange>>,
        after: Vec<LedgerEntryChange>,
    ) -> TransactionMeta {
        let operations: Vec<OperationMeta> = ops
            .into_iter()
            .map(|c| OperationMeta {
                changes: changes(c),
            })
            .collect();
        TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: changes(before),
            operations: operations.try_into().unwrap(),
            tx_changes_after: changes(after),
            soroban_meta: None,
        })
    }

    fn legacy() -> TransactionMeta {
        TransactionMeta::V0(VecM::default())
    }

    /// The ordering contract: before · op0 · op1 · after. Consumers telescope
    /// before→after images, so a reordering here would silently change results.
    #[test]
    fn changes_come_in_before_then_ops_then_after_order() {
        let meta = v3(
            vec![removed(0xB1)],
            vec![vec![removed(0xC1)], vec![removed(0xC2)]],
            vec![removed(0xA1)],
        );
        let tags: Vec<u8> = ledger_changes(&meta).iter().map(|c| tag(c)).collect();
        assert_eq!(tags, vec![0xB1, 0xC1, 0xC2, 0xA1]);
    }

    #[test]
    fn changes_empty_for_legacy() {
        assert!(ledger_changes(&legacy()).is_empty());
    }
}
