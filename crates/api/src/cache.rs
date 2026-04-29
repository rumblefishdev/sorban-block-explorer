//! Shared cache helpers for the API crate.
//!
//! All in-process caches go through `moka::sync::Cache` via the
//! [`ttl_cache`] builder so every module gets the same defaults
//! (TTL + bounded `max_capacity`) without copy-pasting the
//! `Arc<Mutex<HashMap>>` + sweep + poison-recovery boilerplate.
//!
//! See `lore/1-tasks/active/0180_REFACTOR_api-cache-moka-migration.md`
//! for the rationale (in particular: why `moka` and why no Redis yet).
//!
//! ## Sync vs future
//!
//! This helper builds `moka::sync::Cache`. It fits the common case where
//! the cache fetch is independent of any single async operation (handler
//! does its `.await`s outside the `get`/`insert` call) — sync is sharded
//! and lock-free for reads with no risk of holding a lock across `.await`.
//!
//! The `network_cache` is the exception: it needs `try_get_with` with an
//! async initialiser (a Postgres query) to deduplicate concurrent
//! cold-cache requests on a singleton key. That cache is built ad-hoc
//! with `moka::future::Cache::builder()` directly — see
//! `crates/api/src/network/cache.rs`. If a second future-cache callsite
//! ever appears, lift it into a `ttl_future_cache` companion helper here.
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
