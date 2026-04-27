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
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::dto::ContractDetailResponse;

/// Lifetime of a cached contract-detail response.
pub const CACHE_TTL: Duration = Duration::from_secs(45);

/// Sweep all expired entries every Nth `put` so a long-lived warm Lambda
/// container does not accumulate unbounded entries under high-cardinality
/// traffic. Lazy per-key eviction on `get` already drops expired keys when
/// they are revisited; this sweep handles keys that are written once and
/// never read again. 64 picks an amortised O(1) overhead while keeping
/// the worst-case map size bounded by `64 × peak QPS` distinct contracts
/// within one TTL window.
const SWEEP_EVERY_N_PUTS: u64 = 64;

/// Process-wide handle to the contract-metadata cache. Cheap to clone
/// (`Arc`-backed) and safe to share across axum handlers.
#[derive(Clone, Default)]
pub struct ContractMetadataCache {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    map: HashMap<String, Entry>,
    /// Counter for the sweep heuristic — incremented on every `put`.
    puts_since_sweep: u64,
}

struct Entry {
    expires_at: Instant,
    payload: Arc<ContractDetailResponse>,
}

impl ContractMetadataCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the inner mutex, recovering transparently from poisoning.
    ///
    /// A panic inside the locked section would otherwise re-panic every
    /// subsequent caller and effectively crash the warm Lambda container.
    /// Our critical sections are `HashMap::get`/`remove`/`insert` plus an
    /// `Instant` comparison — none of which panic in practice — but this
    /// is cheap insurance: on poisoning we log a warn, take the inner data
    /// as-is, and treat the cache as a soft-state store.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| {
            tracing::warn!(
                "contract cache mutex was poisoned by a prior panic; \
                 continuing with the surviving state"
            );
            poisoned.into_inner()
        })
    }

    /// Return the cached detail when present and unexpired. Expired entries
    /// are removed in-line so the next caller sees a clean miss.
    pub fn get(&self, contract_id: &str) -> Option<Arc<ContractDetailResponse>> {
        let mut guard = self.lock();
        match guard.map.get(contract_id) {
            Some(entry) if entry.expires_at > Instant::now() => Some(Arc::clone(&entry.payload)),
            Some(_) => {
                guard.map.remove(contract_id);
                None
            }
            None => None,
        }
    }

    /// Insert a freshly-fetched detail under `contract_id` and return the
    /// shared `Arc` so the caller can serialize without an extra clone.
    ///
    /// Triggers a full expired-entry sweep every
    /// [`SWEEP_EVERY_N_PUTS`] writes to bound memory under
    /// write-heavy / read-light traffic patterns.
    pub fn put(
        &self,
        contract_id: String,
        payload: ContractDetailResponse,
    ) -> Arc<ContractDetailResponse> {
        let payload = Arc::new(payload);
        let mut guard = self.lock();
        guard.map.insert(
            contract_id,
            Entry {
                expires_at: Instant::now() + CACHE_TTL,
                payload: Arc::clone(&payload),
            },
        );
        guard.puts_since_sweep = guard.puts_since_sweep.saturating_add(1);
        if guard.puts_since_sweep >= SWEEP_EVERY_N_PUTS {
            let now = Instant::now();
            guard.map.retain(|_, e| e.expires_at > now);
            guard.puts_since_sweep = 0;
        }
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
        cache.inner.lock().unwrap().map.insert(
            "CDEF".into(),
            Entry {
                expires_at: Instant::now() - Duration::from_secs(1),
                payload,
            },
        );
        assert!(cache.get("CDEF").is_none());
        assert!(!cache.inner.lock().unwrap().map.contains_key("CDEF"));
    }

    #[test]
    fn full_sweep_clears_expired_entries_after_n_puts() {
        let cache = ContractMetadataCache::new();
        // Insert one entry that is already expired.
        cache.inner.lock().unwrap().map.insert(
            "EXPIRED".into(),
            Entry {
                expires_at: Instant::now() - Duration::from_secs(1),
                payload: Arc::new(sample("EXPIRED")),
            },
        );
        // Drive `put` until the sweep threshold is reached.
        for i in 0..SWEEP_EVERY_N_PUTS {
            cache.put(format!("FRESH{i}"), sample(&format!("FRESH{i}")));
        }
        // The expired entry should have been swept by the last `put`.
        assert!(!cache.inner.lock().unwrap().map.contains_key("EXPIRED"));
        // Fresh entries written within this window are still present.
        assert!(
            cache
                .inner
                .lock()
                .unwrap()
                .map
                .contains_key(&format!("FRESH{}", SWEEP_EVERY_N_PUTS - 1))
        );
    }

    #[test]
    fn poisoned_lock_does_not_panic_subsequent_callers() {
        use std::sync::Arc as StdArc;
        use std::thread;
        let cache = ContractMetadataCache::new();
        cache.put("KEEP".into(), sample("KEEP"));
        // Poison the mutex by panicking inside the locked section.
        let cache_for_panic = cache.clone();
        let _ = thread::spawn(move || {
            let _guard = cache_for_panic.inner.lock().unwrap();
            panic!("intentional poison");
        })
        .join();
        assert!(StdArc::strong_count(&cache.inner) >= 1);
        // After poisoning, gets and puts must keep working.
        let hit = cache.get("KEEP").expect("must recover from poison");
        assert_eq!(hit.contract_id, "KEEP");
        cache.put("AFTER".into(), sample("AFTER"));
        assert!(cache.get("AFTER").is_some());
    }
}
