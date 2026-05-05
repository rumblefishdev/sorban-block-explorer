//! Shared cache helpers for the API crate.
//!
//! Most in-process caches go through this [`ttl_cache`] builder over
//! `moka::sync::Cache` so every module gets the same defaults (TTL and
//! bounded `max_capacity`) without copy-pasting the `Arc<Mutex<HashMap>>`,
//! sweep heuristic and poison-recovery boilerplate. The one exception
//! is the `network_cache`, which uses `moka::future::Cache` for
//! stampede protection on an async initialiser — see the "Sync vs
//! future" section below.
//!
//! See `lore/1-tasks/archive/0180_REFACTOR_api-cache-moka-migration.md`
//! for the rationale (in particular: why `moka` and why no Redis yet).
//!
//! ## Sync vs future
//!
//! This helper builds `moka::sync::Cache`. It fits the common case where
//! the cache fetch is independent of any single async operation (handler
//! does its `.await`s outside the `get`/`insert` call) — sync is sharded
//! and lock-free for reads with no risk of holding a lock across `.await`.
//!
//! When a callsite needs `try_get_with` on an async initialiser to
//! deduplicate concurrent cold-cache misses, use [`ttl_future_cache`]
//! instead. The first such caller was the network stats cache (one
//! singleton key, per task 0180); SEP-1 (task 0188) is the second.
//!
//! ## Capacity defaults
//!
//! Callers must pick `max_capacity` explicitly. The previous
//! `Arc<Mutex<HashMap>>` caches were unbounded, so a high-cardinality
//! scrape could balloon Lambda memory; making the bound explicit at the
//! call site forces every new cache to think about its worst case.

use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

pub use moka::future::Cache as FutureCache;
pub use moka::sync::Cache;

/// Build a TTL-only cache with an explicit max-entry bound.
///
/// Values are wrapped in `Arc<V>` so cache hits clone an `Arc` (one
/// atomic refcount bump) instead of the underlying payload.
///
/// `K` is the cache key; `V` is the cached value type. Both must be
/// `Send + Sync + 'static` so the cache can be shared across handler
/// tasks via `AppState`.
pub fn ttl_cache<K, V>(ttl: Duration, max_capacity: u64) -> Cache<K, Arc<V>>
where
    K: Hash + Eq + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    Cache::builder()
        .time_to_live(ttl)
        .max_capacity(max_capacity)
        .build()
}

/// `moka::future::Cache` companion to [`ttl_cache`] for callers that need
/// `try_get_with` with an `async` initialiser — i.e. cold-miss stampede
/// protection where the load fn is itself a future.
pub fn ttl_future_cache<K, V>(ttl: Duration, max_capacity: u64) -> FutureCache<K, Arc<V>>
where
    K: Hash + Eq + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    FutureCache::builder()
        .time_to_live(ttl)
        .max_capacity(max_capacity)
        .build()
}
