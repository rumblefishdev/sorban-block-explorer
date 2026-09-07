//! The house last-wins fold, shared by every per-ledger collapse.
//!
//! Four staging paths collapse rows to ONE per key with last-in-apply-order
//! semantics (classic pool snapshots, pool-instance images, pool state
//! changes) — the same 16 lines were hand-copied per type until this generic
//! replaced them. Input order IS apply order everywhere it is used:
//! `process.rs` extends vectors while walking transactions in their ledger
//! positions, and every extractor preserves change order within a transaction.
//!
//! The `ledger_sequence` component in each caller's key never varies today —
//! `ParseOutput` and `prepare` handle ONE ledger — and is kept as
//! belt-and-braces: a future caller that batches ledgers would otherwise
//! collapse a pool's two ledgers into one, silently.

use std::collections::HashMap;
use std::hash::Hash;

/// Collapse `items` to one per `key`, keeping the LAST item in input order
/// at the position of the first — order of first appearance is preserved,
/// which keeps the result deterministic for row-level tests.
pub fn keep_last_by_key<T, K: Eq + Hash>(items: Vec<T>, key: impl Fn(&T) -> K) -> Vec<T> {
    use std::collections::hash_map::Entry;
    let mut position: HashMap<K, usize> = HashMap::new();
    let mut folded: Vec<T> = Vec::with_capacity(items.len());
    for item in items {
        match position.entry(key(&item)) {
            Entry::Occupied(e) => folded[*e.get()] = item,
            Entry::Vacant(e) => {
                e.insert(folded.len());
                folded.push(item);
            }
        }
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_item_per_key_at_first_position() {
        let folded = keep_last_by_key(vec![("a", 1), ("b", 2), ("a", 3)], |(k, _)| *k);
        assert_eq!(folded, vec![("a", 3), ("b", 2)]);
    }

    #[test]
    fn distinct_keys_pass_through_in_order() {
        let folded = keep_last_by_key(vec![1, 2, 3], |v| *v);
        assert_eq!(folded, vec![1, 2, 3]);
    }
}
