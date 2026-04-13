---
id: '0137'
title: 'Indexer: DB write performance profiling and backfill-bench improvements'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0117']
tags: [priority-high, effort-medium, layer-db, layer-indexer]
milestone: 1
links:
  - crates/db/src/persistence.rs
  - crates/db/src/soroban.rs
  - crates/indexer/src/handler/persist.rs
  - crates/backfill-bench/src/main.rs
history:
  - date: '2026-04-13'
    status: backlog
    who: fmazur
    note: >
      Spawned from backfill-bench results (task 0117) and senior review.
      Current: ~450ms DB write per ledger on local Postgres (50ms indexing + 450ms persist).
      Target: <100ms DB write. Senior feedback: auto-increment for uniques where possible,
      eliminate redundant unique constraints during backfill.
      Context: local backfill is the production path (Lambda deferred).
  - date: '2026-04-13'
    status: active
    who: fmazur
    note: 'Activated — starting with Phase 0 instrumentation.'
  - date: '2026-04-13'
    status: active
    who: fmazur
    note: >
      Phase 0 complete. Per-query instrumentation added, EXPLAIN ANALYZE run,
      schema layers benchmark done. Key finding: FK triggers are #1 bottleneck
      (85ms/ledger, 24%). Total constraint overhead: 158ms (44%). Even with all
      constraints+indexes dropped, floor is 202ms — <100ms not achievable without
      COPY protocol or parallel workers. Senior decision: stay on BASELINE schema,
      no constraint changes. Backfill-bench rewritten to partition-based S3 sync
      with pipelined download.
  - date: '2026-04-13'
    status: completed
    who: fmazur
    note: >
      Phase 0 profiling complete. 4 files changed: persist.rs (timing instrumentation),
      process.rs (commit_ms), backfill-bench/main.rs (partition-based S3 sync pipeline),
      README.md (minor). 3 research notes produced. Schema optimization deferred —
      senior decision to stay on BASELINE. Backfill-bench rewritten with --cleanup flag
      and pipelined partition downloads.
---

# Indexer: DB write performance profiling and backfill-bench improvements

## Summary

Profiled DB write performance for backfill pipeline. Identified bottlenecks
via per-query instrumentation, EXPLAIN ANALYZE, and incremental schema layer
benchmarks. Rewrote backfill-bench from per-file download to partition-based
`aws s3 sync` with pipelined indexing.

**Context:** Local backfill is the production ingestion path — Lambda-based
ingestion is deferred. Production run scope: ~11M ledgers (hundreds of millions
of rows in events/operations).

## What was done

### 1. Per-query timing instrumentation

Added `Instant::now()` / `elapsed()` wrapping for all 13 DB calls in
`persist_ledger`. Each ledger logs a full breakdown:

```
persist breakdown: insert_ledger=0.5ms | insert_transactions=70ms | insert_operations=28ms | ...
```

Separate `commit_ms` timer added to `process_ledger` to isolate WAL flush cost.

### 2. EXPLAIN ANALYZE on top 3 queries

Ran EXPLAIN (ANALYZE, BUFFERS) on insert_events, insert_transactions,
insert_operations. Key finding: **FK constraint triggers fire per-row** and
dominate cost for events (45% of insert time).

Full analysis: [notes/R-phase0-explain-analysis.md](notes/R-phase0-explain-analysis.md)

### 3. Schema layers benchmark

Incrementally dropped FK → UNIQUE → GIN+B-tree indexes and benchmarked each:

| Layer                          | Avg/ledger | Delta | Savings |
| ------------------------------ | ---------- | ----- | ------- |
| BASELINE                       | 360ms      | —     | —       |
| NO FK                          | 275ms      | -85ms | 24%     |
| NO FK + NO UNIQUE              | 238ms      | -37ms | 10%     |
| NO FK + NO UNIQUE + NO INDEXES | 202ms      | -36ms | 10%     |

**202ms is the floor** — pure heap writes + PK + JSONB parsing. <100ms target
is not achievable through schema changes alone.

Full results: [notes/R-phase0-schema-layers-benchmark.md](notes/R-phase0-schema-layers-benchmark.md)

**Senior decision:** Stay on BASELINE schema. No constraint changes for now.

### 4. Backfill-bench rewrite

Rewrote download pipeline from per-file `aws s3 cp` (spawning 100s of CLI
processes) to partition-based `aws s3 sync`:

- Downloads whole S3 partitions (64k files, ~12.5GB each) in one CLI call
- Pipeline: download partition N+1 in background while indexing partition N
- Skip logic: already-downloaded partitions are not re-synced
- `--cleanup` flag to delete files after indexing (default: keep for reuse)
- Separate download/index timing in final report

### 5. Task plan revised for production context

Updated plan to account for 11M ledger production run:

- Removed `wal_level = minimal` (requires PG restart)
- GIN index drop/rebuild gated on measurement data (rebuild cost: hours at 11M)
- Deferred FK moved to Phase 1 (later found to be no-op — FKs not DEFERRABLE)
- Added rebuild cost warnings for Phase 2 schema changes

## Acceptance Criteria

- [x] Per-query instrumentation in `persist_ledger` with timing logs
- [x] Timing report from Phase 0 with measured breakdown
- [x] EXPLAIN ANALYZE on top 3 queries (events, transactions, operations)
- [x] Schema layers benchmark (FK / UNIQUE / indexes measured independently)
- [x] Production context (11M ledgers) factored into plan
- [x] Backfill-bench rewritten to partition-based S3 sync with pipeline
- [x] Senior review: decision to stay on BASELINE schema documented

## Design Decisions

### From Plan

1. **Measure-first approach**: Phase 0 instrumentation before any code changes.
   Validated by results — original bottleneck assumptions were partially wrong
   (FK triggers, not GIN indexes, were the #1 cost).

2. **Production-safe config only**: Excluded `wal_level = minimal` (requires PG
   restart). All proposed PG tuning was runtime-settable.

### Emerged

3. **`ON CONFLICT DO NOTHING` without constraint names**: Changed queries to use
   generic `ON CONFLICT DO NOTHING` instead of `ON CONFLICT ON CONSTRAINT <name>`
   during benchmarking. Reverted after — constraint names are more explicit and
   provide better error messages.

4. **Partition-based S3 sync over per-file download**: Original backfill-bench
   spawned one `aws s3 cp` per ledger (100s of processes). Switched to
   `aws s3 sync` for entire 64k-file partitions. Trade-off: downloads more data
   than needed for small ranges, but drastically faster for production 11M runs
   and enables reuse across benchmark iterations.

5. **Keep files after indexing by default**: Added `--cleanup` flag instead of
   always deleting. Rationale: partition data (~12.5GB) is reusable across
   multiple benchmark runs, and re-downloading is expensive (minutes per partition).

## Issues Encountered

- **FK constraints not DEFERRABLE**: `SET CONSTRAINTS ALL DEFERRED` was planned
  as a quick win. Subagent review discovered all FKs are defined without
  `DEFERRABLE` — the command is silently a no-op. Would require ALTER TABLE
  to make effective (schema change, not quick win).

- **202ms floor**: Even with all constraints and indexes dropped, insert cost
  is 202ms/ledger. This is pure heap writes + PK B-tree + JSONB serialization
  for ~400 tx/ledger. <100ms target is physically impossible without changing
  the insert mechanism (COPY protocol) or parallelizing across ledgers.

- **idx_timer included download time**: Initial implementation started the index
  timer before partition download, inflating avg/ledger to 17931ms. Fixed by
  moving timer start to after first partition download completes.

## Not done (deferred)

- No-op UPDATE fix for transactions — only helps on replay, not fresh backfill
- Contract interface batching — 0ms in benchmarked range (no WASM uploads)
- PG config tuning (`synchronous_commit = off`) — ~10-15ms gain, marginal
- <100ms target — floor is 202ms; requires COPY protocol or parallel workers
