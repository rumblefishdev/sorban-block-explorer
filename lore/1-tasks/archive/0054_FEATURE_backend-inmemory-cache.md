---
id: '0054'
title: 'Backend: in-memory caching in Lambda execution environment'
type: FEATURE
status: superseded
related_adr: ['0005']
related_tasks: ['0023', '0045', '0050', '0092', '0180']
tags: [layer-backend, caching, lambda, performance]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: axum → Rust (axum + utoipa + sqlx)'
  - date: 2026-04-29
    status: superseded
    who: karolkow
    by: ['0045', '0050', '0180']
    note: >
      Scope absorbed by other tasks. Network stats cache shipped via
      0045 (Network module) and contract metadata cache via 0050
      (Contracts module) as hand-rolled `Arc<Mutex<HashMap>>` with
      TTL. 0180 then refactored both to `moka` 0.12 with max_capacity
      and stampede protection (`try_get_with`). All 0054 acceptance
      criteria satisfied: in-memory TTL cache for network stats and
      contract metadata, persists across warm Lambda invocations,
      lost on cold start, no ElastiCache, no shared cache across
      instances, fall-through to DB on miss. 0054 was never picked
      up as a standalone task — its goals were realized inline in
      the module work. Body of this task still references "Node.js
      single-threaded" — a leftover from the pre-ADR-0005 NestJS
      draft, not corrected because the task is being archived.
---

# Backend: in-memory caching in Lambda execution environment

## Summary

Implement in-memory caching in the Lambda execution environment for two specific cache targets: network stats and frequently accessed contract metadata. This cache uses module-level variables that persist across warm Lambda invocations with 30-60s TTL. No shared cache across instances. No ElastiCache in the initial architecture.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Backlog

**Current state:** Not started. Depends on task 0023 (API bootstrap).

## Context

The Lambda execution environment allows module-level variables to persist across warm invocations within the same instance. This is used for short-TTL caching of frequently accessed, slowly changing data to reduce database round-trips. The cache is lost on cold start, which is acceptable for short-TTL data.

### API Specification

**Location:** `crates/api/src/common/cache/`

### Cache Targets

Only two data types are cached in-memory:

| Target            | TTL    | Rationale                                                     |
| ----------------- | ------ | ------------------------------------------------------------- |
| Network stats     | 30-60s | Called frequently by explorer dashboard, changes slowly       |
| Contract metadata | 30-60s | Popular contracts queried repeatedly, metadata rarely changes |

### What NOT to Cache In-Memory

| Target             | Reason                              | Alternative                                  |
| ------------------ | ----------------------------------- | -------------------------------------------- |
| Transaction detail | Too many unique items, low hit rate | API Gateway caching (long TTL for finalized) |
| Ledger detail      | Same as above                       | API Gateway caching                          |
| List endpoints     | Variable params reduce hit rate     | API Gateway caching                          |
| Search results     | Highly variable                     | No caching                                   |

### Cache Behavior

```
Request → Check in-memory cache (module-level variable)
  → Cache hit + not expired → Return cached value
  → Cache miss or expired → Query DB → Store in cache → Return value
```

**Response shape (internal, not a public API):**

```typescript
interface CacheEntry<T> {
  data: T;
  cached_at: number; // timestamp in ms
  ttl_ms: number; // TTL in milliseconds
}
```

### Behavioral Requirements

- Module-level variables persisting across warm Lambda invocations
- Lost on cold start (acceptable)
- No shared cache across Lambda instances (each instance has its own)
- No ElastiCache or external cache service in initial architecture
- TTL-based invalidation only; no explicit invalidation mechanism
- 30-60s TTL (configurable per cache target)
- Thread-safe within single Lambda invocation (Node.js single-threaded, so inherently safe)

### Caching

This task IS the caching implementation. TTLs:

- Network stats: 30-60s
- Contract metadata: 30-60s

### Error Handling

- Cache failures (if any) must not block the request
- On cache error, fall through to database query
- Cache errors logged but not surfaced to client

## Implementation Plan

### Step 1: Cache Service

Create `crates/api/src/common/cache/` with a simple in-memory cache service. The service manages module-level Map or object storing cached values with timestamps and TTLs.

### Step 2: Network Stats Integration

Integrate cache with the Network module (task 0045). Wrap the stats query so it checks cache first, falls through to DB on miss.

### Step 3: Contract Metadata Integration

Integrate cache with the Contracts module (task 0050). Cache contract metadata lookups for frequently accessed contract IDs.

### Step 4: TTL Configuration

Make TTL configurable (environment variable or constant) with default of 30-60 seconds.

### Step 5: Cache Metrics (Optional)

Add simple logging for cache hit/miss rates to aid operational tuning.

## Acceptance Criteria

- [ ] In-memory cache service with TTL-based expiration
- [ ] Network stats cached with 30-60s TTL
- [ ] Contract metadata cached with 30-60s TTL
- [ ] Cache uses module-level variables (persists across warm invocations)
- [ ] Cache lost on cold start (acceptable, verified by design)
- [ ] No shared cache across Lambda instances
- [ ] No ElastiCache dependency
- [ ] TTL-based invalidation only, no explicit invalidation
- [ ] Cache failures do not block requests (fall through to DB)
- [ ] Transaction/ledger detail NOT cached in-memory (API Gateway handles those)

## Notes

- This is intentionally simple. Complex caching is deferred to API Gateway layer.
- The cache service should be injectable into modules that need it, not a global interceptor.
- Cold start cache misses are expected and acceptable; the data is fast to query from DB.
- If Lambda concurrency is high, each instance maintains its own independent cache.
