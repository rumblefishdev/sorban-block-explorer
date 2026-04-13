---
id: '0136'
title: 'Indexer: optimize DB write performance for backfill (<100ms target)'
type: FEATURE
status: backlog
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
---

# Indexer: optimize DB write performance for backfill

## Summary

Backfill benchmark (task 0117): ~450ms DB write per ledger, ~50ms XDR parse.
Target: <100ms DB writes. Approach: measure first, then optimize in impact order.

**Context:** Local backfill is the production ingestion path — Lambda-based
ingestion is deferred. All optimizations must produce correct, complete data.
Temporary shortcuts (drop indexes/constraints) are acceptable only during the
initial bulk load phase, with mandatory restoration and validation after.

Full root cause analysis: [notes/R-root-cause-analysis.md](notes/R-root-cause-analysis.md)

## Implementation Plan

### Phase 0: Instrument and measure

1. Add per-query timing to `persist_ledger` (wrap each DB call with
   `Instant::now()` / `elapsed()`, log breakdown per ledger)
2. Run backfill-bench on reference range (62015000-62015999), collect per-query
   avg/p50/p95/max, `EXPLAIN ANALYZE` on heaviest queries
3. Publish timing report — validates which bottlenecks actually dominate

### Phase 1: Quick wins (no schema changes, ordered by impact/effort)

4. **PG config tuning** — `synchronous_commit = off`, `wal_level = minimal`,
   `checkpoint_completion_target = 0.9`, `work_mem = 256MB`. Zero code changes.
   Safe for backfill: idempotent re-run covers any crash-lost commits.
   Restore defaults after bulk load completes.
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
8. **Bulk load phase only — `--fast` flag:**
   - Drop GIN indexes before run (`operations.details`, `soroban_events.topics`,
     `soroban_contracts.search_vector`), recreate with `CREATE INDEX CONCURRENTLY`
     after. No API should query during bulk load.
   - `SET CONSTRAINTS ALL DEFERRED` per transaction (FK check at COMMIT,
     not per-INSERT)
   - Flag must print clear warnings: "GIN indexes dropped — do not query until
     backfill completes"

**Benchmark after Phase 1. If <100ms achieved, stop here.**

### Phase 2: Schema changes (only if Phase 1 insufficient)

9. **Drop UNIQUE constraints** on business keys (`uq_events_tx_index`,
   `uq_invocations_tx_index`, `uq_operations_tx_order`) during bulk load only.
   Keep BIGSERIAL as sole PK (monotonic, fast append — senior's recommendation).
   Idempotency: `insert_ledger` returns `rows_affected = 0` for already-processed
   ledgers — skip entire ledger on replay. Re-add constraints + validate after
   bulk load (`ALTER TABLE ... VALIDATE CONSTRAINT` catches any duplicates).
10. If still insufficient, evaluate **COPY protocol** vs INSERT...UNNEST
    (requires persist layer refactor — separate task).
11. Re-benchmark.

### Phase 3: Validate

12. Re-run backfill-bench on reference range (62015000-62015999)
13. Compare: target <100ms avg per ledger
14. Verify idempotency: replay same range, zero duplicates
15. Restore PG config defaults, re-add indexes/constraints if dropped
16. Validate data integrity: row counts, FK consistency, unique constraint check

## Acceptance Criteria

- [ ] Per-query instrumentation in `persist_ledger` with timing logs
- [ ] Timing report from Phase 0 with measured breakdown
- [ ] PG config tuning documented (what to set during bulk load, what to restore after)
- [ ] No-op UPDATE eliminated from `insert_transactions_batch` (permanent fix)
- [ ] Contract interface writes batched, no per-item loop (permanent fix)
- [ ] Multi-ledger batching supported (`--batch-size N`)
- [ ] `--fast` flag for bulk load: drops GIN indexes + defers FK constraints
- [ ] Post-bulk-load restoration: indexes rebuilt, constraints re-added, PG config restored
- [ ] Benchmark: <100ms avg DB write per ledger on local PostgreSQL
- [ ] Idempotency preserved: replay produces zero duplicates, no errors
- [ ] Data integrity validated after bulk load (FK consistency, unique constraints)
- [ ] Schema changes (Phase 2) gated on Phase 0/1 measurement data
