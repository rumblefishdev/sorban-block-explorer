---
id: '0180'
title: 'REFACTOR: migrate API in-memory caches from Arc<Mutex<HashMap>> to moka'
type: REFACTOR
status: completed
related_adr: []
related_tasks: ['0050', '0045', '0047']
tags: [api, cache, refactor, dx]
links: []
history:
  - date: 2026-04-29
    status: active
    who: karolkow
    note: 'Task created'
  - date: 2026-04-29
    status: completed
    who: karolkow
    note: >
      Migration shipped. contracts/cache.rs 218→69, network/cache.rs
      120→66, new shared cache.rs 49. Net cache LOC 338→184 (−154,
      ~45 %). 87 tests pass, 5 ignored. Workspace clippy clean.
      moka 0.12 (sync + future). network_cache uses
      moka::future::Cache::try_get_with for stampede protection on
      /v1/network/stats. Branch 0047 (ledgers) merged into the same
      branch and ships in one PR. Ledger in-process cache evaluated
      and dropped — header `Cache-Control: max-age=300` + AGW are
      sufficient; backlog task can revisit if AGW stays disabled.
---

# REFACTOR: migrate API in-memory caches from Arc<Mutex<HashMap>> to moka

## Summary

Replace two hand-rolled `Arc<Mutex<HashMap>>` cache modules in the API crate
(`crates/api/src/contracts/cache.rs`, `crates/api/src/network/cache.rs`,
~338 lines combined) with a thin wrapper over the `moka` crate. Adds the
missing `max_capacity` bound, enables stampede protection via `try_get_with`,
and provides a single shared helper so future modules stop copy-pasting the
pattern. No ADR — pure refactor; library-choice rationale and the
"no Redis until X" criteria live in this task's `Notes` section and
archive with the task.

## Status: Completed

**Current state:** Shipped on `refactor/0180_api-cache-moka-migration`,
merged with the open ledgers branch (0047). PR opening next.

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
- **Pattern duplication.** Each new module re-implements the same primitives
  with no shared abstraction.

`moka` (the de-facto Rust port of Java's Caffeine, used by SurrealDB,
Materialize, sccache) gives TTL, TTI, max-capacity, TinyLFU eviction,
sharded lock-free reads, and `try_get_with` stampede protection out of the
box. Migration is local to the API crate, requires no infra changes, and
keeps the public type names of the existing caches stable.

Redis / ElastiCache is intentionally **out of scope** — criteria for
revisiting are in `Notes` below.

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

### Step 4: Merge open PR scope

- Branch 0047 (backend ledgers module) has an open PR. Merge it into this
  task's branch so the moka migration also covers any cache code introduced
  by ledgers, and we ship a single coherent caching pass instead of
  refactoring twice.

## Acceptance Criteria

- [x] `moka` added to `crates/api/Cargo.toml`; `cargo build -p api` clean.
- [x] `crates/api/src/cache.rs` exposes a single `ttl_cache` helper.
- [x] `contracts/cache.rs` and `network/cache.rs` reduced to type alias +
      builder call; custom `Mutex<HashMap>`/sweep/poison/`OnceLock`
      code deleted.
- [x] `max_capacity` set on every cache (no unbounded maps).
- [x] `network_cache` uses `moka::future::Cache::try_get_with` for
      stampede protection on the cold-cache path of `/v1/network/stats`.
- [x] All existing API integration tests pass unchanged
      (87 passed, 5 ignored — `cargo test -p api`); workspace clippy
      clean (`cargo clippy --workspace -- -D warnings`).
- [x] Branch 0047 merged into this task's branch.
- [ ] Combined PR opened against `develop`.
- [x] **Code volume reduction recorded** — `contracts/cache.rs` 218 → 69,
      `network/cache.rs` 120 → 66, plus new shared helper `cache.rs` 49.
      Net cache-LOC: 338 → 184, **−154 lines / ≈ 45 % reduction**.
      Target (≥ 100 net delete) met.
- [x] **Docs updated** — `docs/architecture/backend/backend-overview.md`
      §8.1 and `docs/architecture/technical-design-general-overview.md`
      §2.4 both updated to reflect the moka-backed implementation;
      ADR 0032 checklist satisfied (architecture-affecting change
      shipped with matching evergreen-doc edits).

## Implementation Notes

Files touched:

- New: `crates/api/src/cache.rs` (49 lines) — `ttl_cache<K, V>(...)` helper.
- Rewrite: `crates/api/src/contracts/cache.rs` 218 → 69 (`pub type` +
  `new_contract_cache()` returning `moka::sync::Cache`).
- Rewrite: `crates/api/src/network/cache.rs` 120 → 66 (`pub type` +
  `new_network_cache()` returning `moka::future::Cache<(), Arc<NetworkStats>>`).
- `crates/api/src/state.rs` — added `network_cache` field. (Per-`AppState`
  cache; the previous global `OnceLock<Mutex<...>>` static is gone.)
- `crates/api/src/contracts/handlers.rs` — `cache.put(...)` →
  `cache.insert(k, Arc::new(v))`. Public type unchanged.
- `crates/api/src/network/handlers.rs` — single `try_get_with` call with
  an async DB initialiser; ok_response borrowed by ref to avoid cloning
  on the hot path. The `TEST_CACHE_MUTEX` global serialisation primitive
  is gone — each test owns its own cache.
- `crates/api/Cargo.toml` — `moka = { version = "0.12", features = ["sync", "future"] }`.
- `crates/api/src/main.rs` and `tests_integration.rs` — call sites use
  the new `new_*_cache()` builders.
- `docs/architecture/backend/backend-overview.md` §8.1 and
  `docs/architecture/technical-design-general-overview.md` §2.4 —
  describe the moka-backed implementation and the shared helper.

Tests: 87 passed, 5 ignored (network-required). Three TTL/sweep/poison
unit tests deleted as redundant — those behaviours are now moka's
responsibility, fuzz-tested upstream.

## Design Decisions

### From Plan

1. **`moka` over alternatives.** Battle-tested in SurrealDB / Materialize /
   sccache / linkerd. No serious alternative for an idiomatic Rust cache
   with TTL + bounded capacity + lock-free reads.
2. **Shared `ttl_cache` helper, not per-module builders.** Forces every
   future cache to think about `max_capacity` and TTL at the call site;
   prevents the previous "copy-paste 100 lines" pattern.
3. **No Redis / ElastiCache.** Captured under "Why moka, not Redis (yet)"
   below with explicit revisit triggers.
4. **No ADR.** Pure refactor (per Stanisław's review): library swap is not
   an architectural decision. Library-choice rationale and Redis triggers
   live with the task and archive with it.

### Emerged

5. **`moka::future::Cache` for `network_cache`.** Original plan said "use
   `moka::sync::Cache` everywhere" because the old hand-rolled cache had
   to fight the "lock held across `.await`" footgun. After re-reading
   moka's docs the future variant is precisely designed for stampede
   protection with async initialisers and does **not** reintroduce that
   footgun. Switched only the network singleton; contracts cache stays
   sync. Mix is intentional, not legacy.
6. **Per-`AppState` `network_cache` instead of a global static.** The old
   impl used `OnceLock<Mutex<...>>` which forced tests to share state and
   serialise via `TEST_CACHE_MUTEX`. Moving the cache into `AppState`
   removed both the global and the test-side mutex, cleaning up
   `network/handlers.rs` tests considerably.
7. **Ledger detail in-process cache evaluated and dropped.** Initial
   reading of `docs/architecture/backend/backend-overview.md` §6 ("highly
   cacheable once closed") suggested adding a third cache. After
   re-reading task 0047, the cacheability requirement is satisfied by
   `Cache-Control: max-age=300` + API Gateway response cache; an
   in-process cache would duplicate semantics and conflict with 0047's
   "no cross-module coupling" stance. Deferred to a backlog task that
   triggers if AGW cache stays disabled and DB pressure / stampede
   shows up in metrics.
8. **Merge 0047 into this branch.** Branch 0047 (backend ledgers) was
   open as a PR; merging into the moka branch lets the cache pass
   include any new module surface in one coherent PR instead of two.

## Issues Encountered

- **Pre-commit hook reset HEAD~1 once.** First commit on the refactor
  succeeded server-side but the hook's linter modified files and reset
  the local HEAD; re-committed cleanly. No semantic change.
- **`moka::future::Cache::try_get_with` error type is `Arc<E>`.** Plan
  glossed over this. Handler now binds the result as
  `Result<Arc<NetworkStats>, Arc<sqlx::Error>>` and propagates the
  inner error verbatim; the cache itself does not memoise failures so
  the next request retries cleanly.
- **Linter / pre-commit hooks repeatedly normalised `use` ordering and
  doc tweaks** under `lore/1-tasks/active/0180_*.md` and a couple of
  source files. All edits were idempotent and harmless; mentioned here
  in case future sessions see similar churn after the same hooks fire.

## Future Work

Captured as backlog tasks (spawned chips), not as prose here:

- "Extend caching to immutable read endpoints" — ledgers detail (when
  AGW cache stays disabled), transactions detail, asset detail, contract
  interface. Out of scope for 0180; doc text in §6 already promises
  this for ledgers.
- "Extract per-module test fixtures into a shared `tests_integration::make_app`."
  `network/handlers.rs` test module still builds its own `fn app(db)`
  — a leftover from when network was the only module. Independent of
  caching; pure test-side cleanup.

## Notes

### Maintainability impact

Beyond the line-count delta, the refactor pays back on every future
edit touching cache code:

- **New modules need 3 lines, not 100+.** Future caches (assets, ledgers,
  tokens, …) call the shared `ttl_cache(...)` helper instead of
  copy-pasting the `Mutex<HashMap>` + sweep + poison pattern. Onboarding
  cost drops to "set TTL and max_capacity".
- **Reviews shrink.** No bespoke TTL math, no sweep heuristic, no poison
  recovery to read past — review focuses on the cache _policy_ (TTL,
  capacity, key shape) instead of the _mechanism_.
- **Bug surface collapses.** TTL/eviction/concurrency bugs become moka's
  problem (battle-tested, fuzz-tested) rather than ours.
- **Tuning becomes one-line.** Switching to TTI, adding a weigher,
  attaching an eviction listener for metrics — all builder calls, no
  re-implementation.
- **Stops the bleed.** Without this refactor every new module would add
  ~100 lines of cache boilerplate; with it, the marginal cost is ~3.

### Implementation hints

- Keep public type names (`ContractMetadataCache`, network equivalent)
  stable so handler call-sites change minimally.
- `moka::sync::Cache` is itself `Arc`-backed and `Clone` is cheap; the
  outer `Arc<Mutex<...>>` wrapper from the old impl is no longer needed.
- Use `moka::sync::Cache` by default. Prefer `moka::future::Cache` only
  where stampede protection (`try_get_with`) is needed and the
  initialiser is async (e.g. a DB query). The future variant is
  designed for exactly this case — it does **not** reintroduce the
  "lock held across `.await`" footgun the old hand-rolled cache had to
  guard against; user code never holds the cache lock across the
  initialiser.
- Binary size impact ≈ +200 KB; first-build time impact ≈ +3 s. Acceptable.

### Why moka, not Redis (yet)

In-memory `moka` is sufficient because:

- Lambda concurrency model: per-instance memory cache is cheap, no VPC
  cold-start penalty, no fixed monthly cost.
- Block-explorer payloads are dominated by **immutable historical data**
  (closed ledgers, finalised transactions) — those are best handled at the
  API Gateway / CloudFront layer, not Redis.
- Mutable hot data (network stats, contract detail) tolerates 30–60 s TTL;
  per-instance hit ratio is acceptable at current QPS.
- No existing use-case requires shared cross-instance state (no rate
  limiting on Redis, no sessions, no dedup queue, no precomputed aggregates
  refreshed by cron).

**Revisit Redis / ElastiCache when any of these become true:**

1. We need cross-instance rate limiting or per-API-key throttling.
2. Per-instance cache hit ratio drops below ~50 % under steady load.
3. TTL needs to exceed ~5 min with cross-instance invalidation
   (e.g. invalidate-on-write).
4. Precomputed aggregates need to be shared across Lambda instances and
   refreshed by a separate worker.
5. Live-update / WebSocket presence requires shared state.

If/when one of these triggers, open a dedicated ADR proposing the
introduction of a shared cache layer. Until then this task captures the
default.
