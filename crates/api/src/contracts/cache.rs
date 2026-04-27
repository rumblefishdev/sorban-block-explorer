//! In-memory contract-metadata cache scoped to the Lambda execution
//! environment. ADR 0029 leaves heavy detail in S3; the contract detail
//! response itself is small and worth caching for 30–60 seconds so that
//! repeated explorer page-views of the same contract avoid re-issuing the
//! detail + stats queries against Postgres.
//!
//! TTL is fixed at 45 seconds (midpoint of the 30–60 s window in task 0050).
//! Eviction is lazy — entries are dropped on a read miss after expiry.
//!
//! ## Synchronisation primitive
//!
//! Uses `std::sync::Mutex` deliberately. The critical section is a
//! `HashMap::get`, an `Instant` comparison, and an optional `HashMap::remove`
//! — microseconds, with no `.await`s held across the lock. Per the Tokio
//! guidance ("It's OK to use a `std::sync::Mutex` from async code as long
//! as the critical section is short and never `.await`s") this is the
//! correct primitive: switching to `tokio::sync::Mutex` or `parking_lot`
//! would only add async overhead or a dependency for no measurable win at
//! Lambda's per-instance concurrency.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::dto::ContractDetailResponse;

/// Lifetime of a cached contract-detail response.
pub const CACHE_TTL: Duration = Duration::from_secs(45);

/// Process-wide handle to the contract-metadata cache. Cheap to clone
/// (`Arc`-backed) and safe to share across axum handlers.
#[derive(Clone, Default)]
pub struct ContractMetadataCache {
    inner: Arc<Mutex<HashMap<String, Entry>>>,
}

struct Entry {
    expires_at: Instant,
    payload: Arc<ContractDetailResponse>,
}

impl ContractMetadataCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached detail when present and unexpired. Expired entries
    /// are removed in-line so the next caller sees a clean miss.
    pub fn get(&self, contract_id: &str) -> Option<Arc<ContractDetailResponse>> {
        let mut guard = self.inner.lock().expect("contract cache mutex poisoned");
        match guard.get(contract_id) {
            Some(entry) if entry.expires_at > Instant::now() => Some(Arc::clone(&entry.payload)),
            Some(_) => {
                guard.remove(contract_id);
                None
            }
            None => None,
        }
    }

    /// Insert a freshly-fetched detail under `contract_id` and return the
    /// shared `Arc` so the caller can serialize without an extra clone.
    pub fn put(
        &self,
        contract_id: String,
        payload: ContractDetailResponse,
    ) -> Arc<ContractDetailResponse> {
        let payload = Arc::new(payload);
        let mut guard = self.inner.lock().expect("contract cache mutex poisoned");
        guard.insert(
            contract_id,
            Entry {
                expires_at: Instant::now() + CACHE_TTL,
                payload: Arc::clone(&payload),
            },
        );
        payload
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(contract_id: &str) -> ContractDetailResponse {
        ContractDetailResponse {
            contract_id: contract_id.to_string(),
            wasm_hash: None,
            deployer_account: None,
            deployed_at_ledger: None,
            contract_type: None,
            is_sac: false,
            metadata: None,
            stats: super::super::dto::ContractStats {
                invocation_count: 0,
                event_count: 0,
            },
        }
    }

    #[test]
    fn miss_then_hit() {
        let cache = ContractMetadataCache::new();
        assert!(cache.get("CABC").is_none());
        cache.put("CABC".into(), sample("CABC"));
        let hit = cache.get("CABC").expect("hit");
        assert_eq!(hit.contract_id, "CABC");
    }

    #[test]
    fn expired_entry_returns_none() {
        let cache = ContractMetadataCache::new();
        // Stuff a manually-aged entry past its expiry.
        let payload = Arc::new(sample("CDEF"));
        cache.inner.lock().unwrap().insert(
            "CDEF".into(),
            Entry {
                expires_at: Instant::now() - Duration::from_secs(1),
                payload,
            },
        );
        assert!(cache.get("CDEF").is_none());
        assert!(cache.inner.lock().unwrap().get("CDEF").is_none());
    }
}
