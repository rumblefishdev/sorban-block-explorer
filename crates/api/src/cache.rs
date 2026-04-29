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
//! ## Why `sync` and not `future`
//!
//! Handlers fetch from Postgres via `.await`, but the cache `get`/`insert`
//! calls themselves are not held across an `.await`. `moka::sync::Cache`
//! is sharded and lock-free for reads — there is no benefit to the
//! `future` variant for our handler shape, and the sync variant cannot
//! accidentally hold a lock across an `.await`.
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
