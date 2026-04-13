---
id: '0136'
title: 'Indexer: optimize DB write performance for backfill (<100ms target)'
type: FEATURE
status: active
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
---

# Indexer: optimize DB write performance for backfill

## Summary

Backfill benchmark (task 0117): ~450ms DB write per ledger, ~50ms XDR parse.
Target: <100ms DB writes. Approach: measure first, then optimize in impact order.

**Context:** Local backfill is the production ingestion path — Lambda-based
ingestion is deferred. Production run scope: ~11M ledgers (hundreds of millions
of rows in events/operations). All optimizations must produce correct, complete
data. At this scale, any "drop and rebuild" strategy (GIN indexes, UNIQUE
constraints) carries significant rebuild cost (hours of `CREATE INDEX` /
`VALIDATE CONSTRAINT`), so such approaches are gated on Phase 0 measurement
data rather than assumed beneficial.

Full root cause analysis: [notes/R-root-cause-analysis.md](notes/R-root-cause-analysis.md)

## Implementation Plan

### Phase 0: Instrument and measure

1. Add per-query timing to `persist_ledger` (wrap each DB call with
   `Instant::now()` / `elapsed()`, log breakdown per ledger)
2. Run backfill-bench on reference range (62015000-62015999), collect per-query
   avg/p50/p95/max, `EXPLAIN ANALYZE` on heaviest queries
3. Publish timing report — validates which bottlenecks actually dominate

### Phase 1: Quick wins (no schema changes, ordered by impact/effort)

4. **PG config tuning** — `synchronous_commit = off`,
   `checkpoint_completion_target = 0.9`, `work_mem = 256MB`. Zero code changes.
   All settable at runtime (`SET` / `ALTER SYSTEM`), no PG restart required.
   Safe for backfill: idempotent re-run covers any crash-lost commits.
   Restore defaults after bulk load completes.
   **Excluded:** `wal_level = minimal` — requires PG restart, disables
   replication, marginal gain when `synchronous_commit = off` already applied.
5. **Fix transactions no-op UPDATE** — replace `ON CONFLICT DO UPDATE...RETURNING`
   with: INSERT DO NOTHING (no RETURNING) + SELECT hash, id WHERE hash = ANY($1).
   Works for both fresh inserts and replays. Avoids WAL writes on conflict.
   This is a permanent code improvement — no rollback needed.
6. **Multi-ledger batching** — add `--batch-size N` to backfill-bench. N ledgers
   per COMMIT reduces overhead. On batch failure, retry one-by-one. Note: if
   `synchronous_commit = off` already eliminates WAL flush cost, batching gives
   diminishing returns — measure PG config impact alone first in Phase 0.
7. **Batch contract interfaces** — replace per-item loop (`persist.rs:179-192`)
   with single UNNEST queries. Eliminates ~20 roundtrips per ledger.
   Permanent code improvement.
8. **Deferred FK constraints** — `SET CONSTRAINTS ALL DEFERRED` per transaction
   (FK check at COMMIT, not per-INSERT). Zero rebuild cost, safety preserved.
   Permanent improvement — no rollback needed.

**Benchmark after Phase 1. If <100ms achieved, stop here.**

**Deferred to Phase 2 (data-gated):** GIN index drop/rebuild. At 11M ledgers
(hundreds of millions of rows), `CREATE INDEX CONCURRENTLY` takes hours and
blocks API usage. Only pursue if Phase 0 shows GIN maintenance is a dominant
cost (>20% of write time).

### Phase 2: Schema changes (only if Phase 1 insufficient)

9. **Drop GIN indexes** — only if Phase 0 proves GIN maintenance >20% of write
   time. Drop before run, `CREATE INDEX CONCURRENTLY` after. Budget hours for
   rebuild at 11M scale. API must be offline during backfill.
10. **Drop UNIQUE constraints** on business keys (`uq_events_tx_index`,
    `uq_invocations_tx_index`, `uq_operations_tx_order`) during bulk load only.
    Keep BIGSERIAL as sole PK (monotonic, fast append — senior's recommendation).
    Idempotency: `insert_ledger` returns `rows_affected = 0` for already-processed
    ledgers — skip entire ledger on replay. Re-add constraints + validate after
    bulk load. **Warning:** `ALTER TABLE ... VALIDATE CONSTRAINT` on hundreds of
    millions of rows is expensive (hours). Only pursue if impact justifies cost.
11. If still insufficient, evaluate **COPY protocol** vs INSERT...UNNEST
    (requires persist layer refactor — separate task).
12. Re-benchmark.

### Phase 3: Validate

13. Re-run backfill-bench on reference range (62015000-62015999)
14. Compare: target <100ms avg per ledger
15. Verify idempotency: replay same range, zero duplicates
16. Restore PG config defaults (`synchronous_commit = on`, default `work_mem`)
17. If indexes/constraints were dropped (Phase 2): rebuild and validate — budget
    hours for this step at production scale
18. Validate data integrity: row counts, FK consistency, unique constraint check

## Acceptance Criteria

- [ ] Per-query instrumentation in `persist_ledger` with timing logs
- [ ] Timing report from Phase 0 with measured breakdown
- [ ] PG config tuning documented (runtime-settable only, no PG restart)
- [ ] No-op UPDATE eliminated from `insert_transactions_batch` (permanent fix)
- [ ] Contract interface writes batched, no per-item loop (permanent fix)
- [ ] Deferred FK constraints per transaction (permanent fix)
- [ ] Multi-ledger batching supported (`--batch-size N`) — gated on Phase 0 data
- [ ] Benchmark: <100ms avg DB write per ledger on local PostgreSQL
- [ ] Idempotency preserved: replay produces zero duplicates, no errors
- [ ] Data integrity validated after bulk load (FK consistency, unique constraints)
- [ ] Schema changes (Phase 2) gated on Phase 0/1 measurement data — with
      rebuild cost explicitly budgeted for 11M ledger scale
