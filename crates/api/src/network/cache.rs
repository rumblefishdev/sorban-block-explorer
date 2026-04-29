//! In-process cache for `GET /v1/network/stats`.
//!
//! Backed by `moka::future::Cache` so the cold-cache path can use
//! `try_get_with` for **stampede protection**: when N concurrent
//! requests arrive on a cold key, only one runs the (async) Postgres
//! query and the rest wait on its result instead of fanning out N
//! round-trips. Per task 0180, this replaces a global
//! `OnceLock<Mutex<...>>` singleton plus a hand-rolled TTL check.
//!
//! `moka::future::Cache::try_get_with` accepts an `async` initialiser,
//! which lets us keep the DB fetch on the async runtime without
//! reintroducing the "lock held across `.await`" footgun the previous
//! impl had to guard against.
//!
//! Per `docs/architecture/backend/backend-overview.md` §8.x backend
//! in-memory caching has a 30–60 s TTL — we settle on 30 s. Two cache
//! layers stack: the API Gateway sits in front (5–15 s mutable TTL,
//! disabled today per `infra/envs/*.json`) and this Lambda layer behind
//! it. Worst-case user-perceived staleness is **additive** across the
//! two layers (~30 s + ~10 s = ~40 s today) — see `network/handlers.rs`
//! for the matching `Cache-Control` header.

use std::sync::Arc;
use std::time::Duration;

use super::dto::NetworkStats;

/// 30-second TTL — within the documented `30-60s` window. Shorter values
/// defeat the purpose; longer ones let the response drift past the API
/// Gateway TTL ceiling on consecutive requests.
const TTL: Duration = Duration::from_secs(30);

/// Single-key cache (the network stats endpoint is a singleton).
const MAX_ENTRIES: u64 = 1;

/// Cache key type for the network stats endpoint. The cache holds one
/// entry, so the key is the unit type — `try_get_with(())` deduplicates
/// concurrent misses without us having to invent a sentinel string.
pub type NetworkStatsCache = moka::future::Cache<(), Arc<NetworkStats>>;

/// Build a fresh cache instance with the canonical TTL.
pub fn new_network_cache() -> NetworkStatsCache {
    moka::future::Cache::builder()
        .time_to_live(TTL)
        .max_capacity(MAX_ENTRIES)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seq: i64) -> NetworkStats {
        use chrono::TimeZone;
        let ts = chrono::Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        NetworkStats {
            tps_60s: 1.5,
            total_accounts: 100,
            total_contracts: 5,
            latest_ledger_sequence: seq,
            latest_ledger_closed_at: Some(ts),
            generated_at: ts,
        }
    }

    #[tokio::test]
    async fn put_then_get_round_trips_within_ttl() {
        let cache = new_network_cache();
        cache.insert((), Arc::new(sample(42))).await;
        let read = cache.get(&()).await.expect("cache populated within TTL");
        assert_eq!(read.latest_ledger_sequence, 42);
        assert_eq!(read.total_accounts, 100);
    }
}
