//! Per-worker cache of `soroban_contracts.contract_type` used by the NFT
//! insert filter (task 0118 Phase 2).
//!
//! The indexer processes many ledgers per Lambda invocation. Without a
//! cache, every ledger would re-query `soroban_contracts` for the same
//! contracts referenced by NFT-candidate events. The cache collapses that
//! to one batch lookup per ledger, hitting only contracts unseen so far.
//!
//! # Cacheable values
//!
//! Only **definitive** classifications are cached:
//!
//! * [`ContractType::Token`]    — SAC pre-classified at deploy
//! * [`ContractType::Nft`]      — WASM exposes NFT discriminators
//! * [`ContractType::Fungible`] — WASM exposes SEP-0041 discriminators
//!
//! [`ContractType::Other`] is **never** cached. Workers must re-query on
//! next encounter so that a later WASM upload (processed by a different
//! worker or later in time) can promote the contract out of `Other`.
//!
//! # Concurrency
//!
//! Lambda invocations are serialized per instance, but `HandlerState` is
//! cloneable and could in principle be shared across futures. A cheap
//! `std::sync::Mutex` suffices — lock contention is effectively zero in
//! practice.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use domain::ContractType;

/// Shared, clone-on-write cache of contract classifications.
#[derive(Clone, Default)]
pub struct ClassificationCache {
    inner: Arc<Mutex<HashMap<String, ContractType>>>,
}

impl ClassificationCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fast path lookup for a single id. Prefer [`Self::snapshot_for`]
    /// when inspecting many ids from a hot loop — one lock round-trip
    /// instead of one per call. `None` means "ask the DB": the entry was
    /// never observed or it was observed as `Other` (deliberately not
    /// cached so promotion can happen later).
    #[allow(dead_code)] // consumed by integration tests + diagnostics
    pub fn get(&self, contract_id: &str) -> Option<ContractType> {
        self.inner
            .lock()
            .expect("classification cache mutex poisoned")
            .get(contract_id)
            .copied()
    }

    /// Bulk insert. `Other` entries are filtered out silently.
    pub fn extend_definitive<I>(&self, entries: I)
    where
        I: IntoIterator<Item = (String, ContractType)>,
    {
        let mut guard = self
            .inner
            .lock()
            .expect("classification cache mutex poisoned");
        for (id, ty) in entries {
            if is_definitive(ty) {
                guard.insert(id, ty);
            }
        }
    }

    /// Collect the `contract_id`s unseen by the cache. Callers issue one
    /// `SELECT … WHERE contract_id = ANY(…)` for the result, then populate
    /// via [`Self::extend_definitive`].
    pub fn missing<'a, I>(&self, ids: I) -> Vec<&'a str>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let guard = self
            .inner
            .lock()
            .expect("classification cache mutex poisoned");
        ids.into_iter()
            .filter(|id| !guard.contains_key(*id))
            .collect()
    }

    /// Take a single lock and read every known verdict for `ids` into a
    /// local map. The returned `HashMap` is then consulted lock-free by
    /// callers making per-row filter decisions — avoids one lock round-trip
    /// per row on large ledgers (task 0118 Phase 2 NFT filter).
    pub fn snapshot_for<'a, I>(&self, ids: I) -> HashMap<&'a str, ContractType>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let guard = self
            .inner
            .lock()
            .expect("classification cache mutex poisoned");
        ids.into_iter()
            .filter_map(|id| guard.get(id).map(|ty| (id, *ty)))
            .collect()
    }
}

/// Whether a `ContractType` value should be cached.
pub(crate) fn is_definitive(ty: ContractType) -> bool {
    matches!(
        ty,
        ContractType::Token | ContractType::Nft | ContractType::Fungible
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_is_not_cached() {
        let cache = ClassificationCache::new();
        cache.extend_definitive(vec![("C_OTHER".into(), ContractType::Other)]);
        assert_eq!(cache.get("C_OTHER"), None);
    }

    #[test]
    fn definitive_values_roundtrip() {
        let cache = ClassificationCache::new();
        cache.extend_definitive(vec![
            ("C_NFT".into(), ContractType::Nft),
            ("C_FUN".into(), ContractType::Fungible),
            ("C_TOK".into(), ContractType::Token),
        ]);
        assert_eq!(cache.get("C_NFT"), Some(ContractType::Nft));
        assert_eq!(cache.get("C_FUN"), Some(ContractType::Fungible));
        assert_eq!(cache.get("C_TOK"), Some(ContractType::Token));
    }

    #[test]
    fn missing_returns_only_uncached_ids() {
        let cache = ClassificationCache::new();
        cache.extend_definitive(vec![("C_NFT".into(), ContractType::Nft)]);
        let misses = cache.missing(["C_NFT", "C_UNKNOWN_1", "C_UNKNOWN_2"]);
        assert_eq!(misses, vec!["C_UNKNOWN_1", "C_UNKNOWN_2"]);
    }

    #[test]
    fn snapshot_for_returns_only_cached_hits() {
        let cache = ClassificationCache::new();
        cache.extend_definitive(vec![
            ("C_NFT".into(), ContractType::Nft),
            ("C_FUN".into(), ContractType::Fungible),
        ]);
        let snap = cache.snapshot_for(["C_NFT", "C_FUN", "C_UNKNOWN"]);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap.get("C_NFT"), Some(&ContractType::Nft));
        assert_eq!(snap.get("C_FUN"), Some(&ContractType::Fungible));
        assert_eq!(snap.get("C_UNKNOWN"), None);
    }

    #[test]
    fn extend_filters_other() {
        let cache = ClassificationCache::new();
        cache.extend_definitive(vec![
            ("C1".into(), ContractType::Nft),
            ("C2".into(), ContractType::Other),
            ("C3".into(), ContractType::Fungible),
        ]);
        assert_eq!(cache.get("C1"), Some(ContractType::Nft));
        assert_eq!(cache.get("C2"), None);
        assert_eq!(cache.get("C3"), Some(ContractType::Fungible));
    }
}
