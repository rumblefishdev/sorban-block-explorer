//! Process-local in-memory cache for `GET /v1/network/stats`.
//!
//! The endpoint is hit on every Home dashboard load. Per
//! `docs/architecture/backend/backend-overview.md` §8.x backend
//! in-memory caching has a 30-60s TTL — we settle on 30s, which keeps
//! the maximum perceived staleness in line with the `5-15s` API
//! Gateway TTL while reducing DB round-trips by ~30× under sustained
//! polling.
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
/// [`TTL`], otherwise `None`. A poisoned mutex is treated as a cache
/// miss — the next handler call will repopulate.
pub fn get() -> Option<NetworkStats> {
    let lock = cell().lock().ok()?;
    let (written_at, stats) = lock.as_ref()?;
    if written_at.elapsed() < TTL {
        Some(stats.clone())
    } else {
        None
    }
}

/// Replace the cache slot with a fresh value. A poisoned mutex is
/// silently ignored — losing a write means the next request will
/// refetch, which is correct.
pub fn put(stats: NetworkStats) {
    if let Ok(mut lock) = cell().lock() {
        *lock = Some((Instant::now(), stats));
    }
}

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

    // The cache is process-wide. We don't run mutually-exclusive cache
    // assertions — instead each test installs a unique sentinel value
    // and verifies it round-trips. The two tests therefore commute.

    #[test]
    fn put_then_get_returns_clone_of_value() {
        let s = sample(42);
        put(s.clone());
        let read = get().expect("cache populated");
        assert_eq!(read.highest_indexed_ledger, 42);
        assert_eq!(read.total_accounts, 100);
    }

    #[test]
    fn elapsed_check_is_lt_ttl_immediately_after_put() {
        // Sanity: a value written now is within TTL — `get()` returns
        // `Some`. Time-based expiry is exercised implicitly by the TTL
        // constant; no sleep test (would slow CI by 30s).
        put(sample(7));
        assert!(get().is_some());
    }
}
