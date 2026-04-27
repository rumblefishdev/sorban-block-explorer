//! Process-local in-memory cache for `GET /v1/network/stats`.
//!
//! The endpoint is hit on every Home dashboard load. Per
//! `docs/architecture/backend/backend-overview.md` §8.x backend
//! in-memory caching has a 30-60s TTL — we settle on 30s. Two cache
//! layers stack: the API Gateway sits in front (5-15s mutable TTL,
//! disabled today per `infra/envs/*.json`) and this Lambda layer
//! behind it.
//!
//! Worst-case user-perceived staleness is **additive** across the two
//! layers, not bounded by the inner TTL: AGW can latch a response
//! that was already near-expired in the Lambda cache, so the ceiling
//! is roughly `inner_ttl + agw_ttl` (~30s + ~10s = ~40s today). If a
//! tighter ceiling is ever required, lower one or both TTLs. The
//! Home dashboard does not have a hard freshness requirement, so the
//! current pair is intentional.
//!
//! Implementation choice: `OnceLock<Mutex<...>>` from std. Lock
//! contention is irrelevant here — the critical section is a single
//! pointer write, and the endpoint has at most one hit per warm Lambda
//! instance per ~30s on the cache-miss path. No external dependency
//! pulled in.
//!
//! The cache survives across warm invocations because the Lambda
//! Tokio runtime is reused; it is intentionally lost on cold start
//! (handler is initialised fresh, the static is re-zeroed).
//!
//! Mutex poisoning is recovered via `PoisonError::into_inner` rather
//! than treated as a permanent cache disable — a panic in some other
//! handler path would otherwise silently kill caching for the lifetime
//! of the process.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::dto::NetworkStats;

/// 30-second TTL — within the documented `30-60s` window. A shorter
/// value would defeat the purpose; a longer one would let the response
/// drift past the API Gateway TTL ceiling on consecutive requests.
const TTL: Duration = Duration::from_secs(30);

/// Lazily-initialised process-wide cache cell. `None` means "no value
/// yet"; `Some((written_at, stats))` is the most recent successful
/// fetch. We never cache errors.
static CACHE: OnceLock<Mutex<Option<(Instant, NetworkStats)>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<(Instant, NetworkStats)>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Return a cached `NetworkStats` clone if one is present and within
/// [`TTL`], otherwise `None`. Recovers from mutex poisoning so a
/// panic in an unrelated path does not permanently disable caching.
pub fn get() -> Option<NetworkStats> {
    let lock = cell().lock().unwrap_or_else(|p| p.into_inner());
    let (written_at, stats) = lock.as_ref()?;
    if written_at.elapsed() < TTL {
        Some(stats.clone())
    } else {
        None
    }
}

/// Replace the cache slot with a fresh value. Recovers from mutex
/// poisoning rather than dropping the write silently.
pub fn put(stats: NetworkStats) {
    let mut lock = cell().lock().unwrap_or_else(|p| p.into_inner());
    *lock = Some((Instant::now(), stats));
}

/// Test-only: drop any cached value so the next `get()` returns
/// `None`. Used by tests in this crate that exercise the global
/// `CACHE` static and need to start from a known-empty slot.
#[cfg(test)]
pub fn clear() {
    let mut lock = cell().lock().unwrap_or_else(|p| p.into_inner());
    *lock = None;
}

/// Test-only mutex serialising every test in this crate that touches
/// the global `CACHE` static (the unit tests below and the handler
/// integration test in `network/handlers.rs`). Without this, parallel
/// test execution can interleave `put()` / `get()` calls and make
/// assertions flaky. Each consumer must:
///
///   let _g = TEST_CACHE_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
///   cache::clear();
///   // ... exercise cache or handler ...
#[cfg(test)]
pub static TEST_CACHE_MUTEX: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seq: i64) -> NetworkStats {
        NetworkStats {
            tps: 1.5,
            total_accounts: 100,
            total_contracts: 5,
            highest_indexed_ledger: seq,
            ingestion_lag_seconds: Some(2),
        }
    }

    #[test]
    fn put_then_get_round_trips_within_ttl() {
        let _guard = TEST_CACHE_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        let s = sample(42);
        put(s.clone());
        let read = get().expect("cache populated within TTL");
        assert_eq!(read.highest_indexed_ledger, 42);
        assert_eq!(read.total_accounts, 100);
    }
}
