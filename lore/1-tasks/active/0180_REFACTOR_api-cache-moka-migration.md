---
id: '0180'
title: 'REFACTOR: migrate API in-memory caches from Arc<Mutex<HashMap>> to moka'
type: REFACTOR
status: active
related_adr: []
related_tasks: ['0050', '0045']
tags: [api, cache, refactor, dx]
links: []
history:
  - date: 2026-04-29
    status: active
    who: karolkow
    note: 'Task created'
---

# REFACTOR: migrate API in-memory caches from Arc<Mutex<HashMap>> to moka

## Summary

Replace two hand-rolled `Arc<Mutex<HashMap>>` cache modules in the API crate
(`crates/api/src/contracts/cache.rs`, `crates/api/src/network/cache.rs`,
~338 lines combined) with a thin wrapper over the `moka` crate. Adds the
missing `max_capacity` bound, enables stampede protection via `try_get_with`,
and provides a single shared helper so future modules stop copy-pasting the
pattern. Also captures the caching strategy in a new ADR.

## Status: Active

**Current state:** Task created. Implementation pending on a fresh branch off `develop`.

## Context

Two API modules currently implement in-memory caches by hand:
`Arc<Mutex<HashMap<String, Entry>>>` plus manual TTL, lazy eviction, a
"sweep every N puts" heuristic, mutex-poison recovery and ~80 lines of tests
per module reimplementing the same TTL/sweep semantics. Real defects:

- **No max capacity.** Map grows unbounded — a high-cardinality scrape
  (e.g. enumerating contracts) can balloon Lambda memory until the next sweep.
- **No stampede protection.** N concurrent requests for the same cold key
  trigger N Postgres round-trips instead of one.
- **Lock contention under burst.** `Mutex<HashMap>` serialises all readers.
- **Pattern duplication.** Each new module re-implements the same primitives;
  there is no shared abstraction and no ADR governing caching choices.

`moka` (the de-facto Rust port of Java's Caffeine, used by SurrealDB,
Materialize, sccache) gives TTL, TTI, max-capacity, TinyLFU eviction,
sharded lock-free reads, and `try_get_with` stampede protection out of the
box. Migration is local to the API crate, requires no infra changes, and
keeps the public type names of the existing caches stable.

Redis / ElastiCache is intentionally **out of scope** — see ADR (below) for
the criteria that would justify introducing a shared cache layer.

## Implementation Plan

### Step 1: Add dependency and shared helper

- Add `moka = { version = "0.12", features = ["sync"] }` to
  `crates/api/Cargo.toml`.
- Create `crates/api/src/cache.rs` with a thin builder helper:

  ```rust
  pub fn ttl_cache<K, V>(ttl: Duration, max: u64) -> moka::sync::Cache<K, Arc<V>>
  where K: Hash + Eq + Send + Sync + 'static,
        V: Send + Sync + 'static;
  ```

- Re-export from `crates/api/src/lib.rs` (or `main.rs`) so all sub-modules
  share one entry point.

### Step 2: Migrate `contracts/cache.rs`

- Replace the `ContractMetadataCache` struct with
  `pub type ContractMetadataCache = moka::sync::Cache<String, Arc<ContractDetailResponse>>;`
  built via the helper (`ttl=45s`, `max_capacity=10_000`).
- Update `contracts/handlers.rs` to use `cache.get(...)` /
  `cache.try_get_with(...)` instead of the bespoke `get`/`put` API.
- Drop the existing tests — TTL/sweep/poison are now moka's responsibility.
  Keep one smoke test that exercises insert + hit + expiry through the
  public handler path.

### Step 3: Migrate `network/cache.rs`

- Same pattern. Pick TTL based on existing constant in `network/cache.rs`.
- Verify the network stats handler's freshness contract
  (`generated_at`) still holds across cache hits.

### Step 4: ADR

- New ADR (next free ID, expected `0040`): "API caching strategy: in-memory
  via moka, no shared cache until concrete cross-instance use-case appears".
- Captures: why moka, why not Redis/ElastiCache yet, criteria that would
  flip the decision (rate limiting, sessions, cross-instance invalidation,
  sustained per-instance hit ratio < 50%), and the API ingress / CloudFront
  layer as the cache-of-record for immutable historical responses.
- Link from this task's `related_adr`.

### Step 5: Merge open PR scope

- Branch 0047 (backend ledgers module) has an open PR. Merge it into this
  task's branch so the moka migration also covers any cache code introduced
  by ledgers, and we ship a single coherent caching pass instead of
  refactoring twice.

## Acceptance Criteria

- [ ] `moka` added to `crates/api/Cargo.toml`; `cargo build -p api` clean.
- [ ] `crates/api/src/cache.rs` exposes a single `ttl_cache` helper.
- [ ] `contracts/cache.rs` and `network/cache.rs` reduced to type alias +
      builder call (~10 lines each); custom `Mutex<HashMap>`/sweep/poison
      code deleted.
- [ ] `max_capacity` set on every cache (no unbounded maps).
- [ ] At least one cache uses `try_get_with` for stampede protection on a
      hot read path.
- [ ] All existing API integration tests pass unchanged.
- [ ] New ADR landed under `lore/2-adrs/` and referenced from this task's
      frontmatter.
- [ ] Branch 0047 merged into this task's branch; combined PR opened
      against `develop`.
- [ ] **Docs updated** — `docs/architecture/backend/backend-overview.md`
      §8.1 (`ContractMetadataCache` description) updated to reflect the
      moka-backed implementation; ADR 0032 checklist filled in.

## Notes

- Keep public type names (`ContractMetadataCache`, network equivalent)
  stable so handler call-sites change minimally.
- `moka::sync::Cache` is itself `Arc`-backed and `Clone` is cheap; the
  outer `Arc<Mutex<...>>` wrapper from the old impl is no longer needed.
- Do **not** introduce `moka::future::Cache` — sync variant is sufficient
  for our handler shape and avoids holding the cache across `.await`.
- Binary size impact ≈ +200 KB; first-build time impact ≈ +3 s. Acceptable.
