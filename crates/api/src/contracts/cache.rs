//! In-process cache for `GET /v1/contracts/:contract_id` responses.
//!
//! Backed by `moka::sync::Cache` via the shared [`crate::cache::ttl_cache`]
//! helper. Per task 0180, this replaces a hand-rolled `Arc<Mutex<HashMap>>`
//! cache with manual TTL, sweep heuristic and poison handling — moka now
//! owns TTL, eviction (TinyLFU), capacity bounds and lock-free reads.
//!
//! TTL is fixed at 45 seconds (midpoint of the 30–60 s window in task 0050)
//! so repeated explorer page-views of the same contract avoid re-issuing
//! the detail + stats queries against Postgres. `MAX_ENTRIES` keeps the map
//! bounded so a high-cardinality scrape cannot balloon Lambda memory.

use std::sync::Arc;
use std::time::Duration;

use crate::cache::{Cache, ttl_cache};

use super::dto::ContractDetailResponse;

/// Lifetime of a cached contract-detail response.
const CACHE_TTL: Duration = Duration::from_secs(45);

/// Bound on distinct contracts cached at once. 10 000 distinct contract
/// detail rows × ~1 KB JSON each ≈ 10 MB worst case, well under Lambda's
/// memory budget and far above realistic working-set size.
const MAX_ENTRIES: u64 = 10_000;

/// Process-wide handle to the contract-metadata cache. `Cache` is itself
/// `Arc`-backed and cheap to clone; share via `AppState`.
pub type ContractMetadataCache = Cache<String, Arc<ContractDetailResponse>>;

/// Build a fresh cache instance with the canonical TTL + capacity.
pub fn new_contract_cache() -> ContractMetadataCache {
    ttl_cache::<String, ContractDetailResponse>(CACHE_TTL, MAX_ENTRIES)
}

#[cfg(test)]
mod tests {
    use super::super::dto::ContractStats;
    use super::*;

    fn sample(contract_id: &str) -> ContractDetailResponse {
        ContractDetailResponse {
            contract_id: contract_id.to_string(),
            wasm_hash: None,
            wasm_uploaded_at_ledger: None,
            deployer: None,
            deployed_at_ledger: None,
            contract_type_name: None,
            contract_type: None,
            is_sac: false,
            metadata: None,
            stats: super::super::dto::ContractStats {
                recent_invocations: 0,
                recent_unique_callers: 0,
                stats_window: "7 days".to_string(),
            },
        }
    }

    /// Smoke test: insert + hit. TTL expiry / eviction / concurrency are
    /// moka's responsibility (and are tested in moka itself); we only
    /// verify the wiring of the type alias + builder helper.
    #[test]
    fn miss_then_hit() {
        let cache = new_contract_cache();
        assert!(cache.get("CABC").is_none());
        cache.insert("CABC".to_string(), Arc::new(sample("CABC")));
        let hit = cache.get("CABC").expect("hit");
        assert_eq!(hit.contract_id, "CABC");
    }
}
