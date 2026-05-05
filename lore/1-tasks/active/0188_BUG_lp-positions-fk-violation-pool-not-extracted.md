---
id: '0188'
title: 'lp_positions FK violation when parent pool not extracted in same ledger'
type: BUG
status: active
related_adr: []
related_tasks: ['0126', '0179']
tags: ['phase-pools', 'effort-medium', 'priority-high', 'indexer', 'extraction']
links: []
history:
  - date: 2026-05-05
    status: backlog
    who: stanislaw
    note: 'Crashed bridge backfill (lore-0185 follow-up) at ledger 62148003 with FK violation lp_positions_pool_id_fkey. Spawned for unit-level repro and fix; partial 132k dataset accepted, no full re-backfill needed.'
  - date: 2026-05-05
    status: active
    who: stanislaw
    note: 'Promoted to active for unit-level repro + fix. Backfill at 132k partial dataset accepted as audit baseline.'
---

# lp_positions FK violation when parent pool not extracted in same ledger

## Summary

Bridge backfill crashed at ledger `62148003` with PostgreSQL `23503` FK
violation: `lp_positions_pool_id_fkey`. The transaction tried to insert an
`lp_positions` row referencing pool
`\xd63184d4e5601fad174d9d5fa8e79f2366f6818892e43867a952e8adb13fa561`, but no
matching row exists in `liquidity_pools` — neither in current-ledger
`staged.pool_rows` nor previously persisted. Indexer extraction emitted an
LP position update without ensuring the parent pool row is staged in the
same persist transaction (or already in DB from a prior ledger).

## Status: Backlog

**Current state:** Reproducer pinned to single ledger. Need unit-level
repro from the raw XDR + targeted fix in extraction or persist staging.

## Context

### Crash

```
thread 'main' panicked at crates/backfill-runner/src/main.rs:105:18:
backfill run failed: Indexer(Database(Database(PgDatabaseError {
  severity: Error,
  code: "23503",
  message: "insert or update on table \"lp_positions\" violates foreign
           key constraint \"lp_positions_pool_id_fkey\"",
  detail: Some("Key (pool_id)=(\\xd63184d4e5601fad174d9d5fa8e79f2366f6818892e43867a952e8adb13fa561)
                is not present in table \"liquidity_pools\"."),
  ...
})))
```

### Reproducer

|            |                                                                                                                 |
| ---------- | --------------------------------------------------------------------------------------------------------------- |
| Ledger     | `62148003`                                                                                                      |
| Pool ID    | `\xd63184d4e5601fad174d9d5fa8e79f2366f6818892e43867a952e8adb13fa561`                                            |
| Source XDR | `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/FC4BC1FF--62144000-62207999/FC4DCC04--62148003.xdr.zst` |
| Local copy | `/Volumes/Extreme SSD 2TB/sbe-backfill-temp/FC4BC1FF--62144000-62207999/FC4DCC04--62148003.xdr.zst`             |

### Persist code path

`crates/indexer/src/handler/persist/write.rs::upsert_pools_and_snapshots`:

1. **13a** `liquidity_pools` INSERT (parent)
2. **13b** `liquidity_pool_snapshots` INSERT
3. **13c** `lp_positions` INSERT (FK → `liquidity_pools.pool_id`)

Sequence is correct within a single tx. Bug is upstream: extractor staged
an `lp_position` row whose `pool_id` is not in `staged.pool_rows` for the
same ledger AND not in DB from a prior persistence.

### Why now

Original 30k validation (lore-0185) didn't hit this — pool didn't appear
in that range. Bridge backfill in range `62,046,001–62,148,002` survived
~102k ledgers before tripping. Pool created/touched somewhere this range,
re-touched at `62148003` without parent row visibility.

## Hypothesis

1. Pool created in earlier ledger but extraction missed pool_creation event
   (silent skip), so DB has lp_position references but no pool row.
2. Pool created in `62148003` itself but extractor emits position before
   pool extraction (ordering bug in `staging.rs`).
3. Extractor sees a deposit/withdraw operation pattern that infers the
   position holder but doesn't backfill the pool dimension row.

Need to inspect raw XDR for ledger `62148003` to determine which.

## Implementation Plan

### Step 1: Repro at unit level

- Read ledger `62148003` XDR from local cache (or re-fetch S3).
- Wire a focused unit/integration test in `crates/indexer` that runs the
  full extraction pipeline on this single ledger, asserting:
  - For every `lp_position_rows[i].pool_id`, either:
    - `staged.pool_rows` contains a row with same `pool_id`, OR
    - `pool_id` already exists in DB pre-test
- Confirm test fails on current code with the same FK pattern.

### Step 2: Diagnose extraction

- Trace pool_id `\xd63184…` through `crates/indexer/src/extract/pools.rs`
  (or wherever LP extraction lives) for ledger `62148003`.
- Identify which operation/event surfaces the position (transfer, deposit,
  withdraw) and why the parent pool row is not also emitted.
- Check: does pool exist in DB from earlier ledger? Query:
  ```sql
  SELECT * FROM liquidity_pools
  WHERE pool_id = '\xd63184d4e5601fad174d9d5fa8e79f2366f6818892e43867a952e8adb13fa561';
  ```

### Step 3: Fix

Two candidate fixes (TBD which):

- **A. Ensure parent emission**: extractor always emits a `pool_rows`
  entry whenever it emits an `lp_position_rows` entry. Use minimal
  placeholder fields (asset metadata, fee_bps) sourced from the same
  evidence trail or fall back to UPSERT NO-OP if parent already exists.
- **B. Defensive persist**: at persist time, dedupe `lp_position_rows`
  whose `pool_id` is not in `staged.pool_rows` AND not in DB. Log skip.
  Less correct (drops data), only acceptable if extraction decision is
  to silently miss certain positions.

A is preferred — preserves data, fixes root cause.

### Step 4: Regression invariant

- Add audit-harness invariant in `crates/audit-harness/sql/`:
  every `lp_positions.pool_id` exists in `liquidity_pools.pool_id`
  (FK should already enforce — formalize as an explicit invariant for
  surfacing if FK gets dropped or DB gets out of sync via raw inserts).
- Add unit test from Step 1 to prevent regression.

## Acceptance Criteria

- [ ] Unit test reproducing FK violation on ledger `62148003`, pool
      `\xd63184…`, fails on current code with current FK semantics
- [ ] Root cause identified — A vs B vs other path
- [ ] Fix implemented — every staged `lp_position_rows` entry has
      corresponding parent in `staged.pool_rows` OR DB
- [ ] Unit test passes after fix
- [ ] No regression in existing pools tests
- [ ] **Docs updated** — `docs/architecture/**` if persist/extraction
      contract for pools changes; `N/A — internal extraction fix`
      otherwise

## Notes

- **Backfill state:** 132k ledgers continuous (62,016k → 62,148,002) in
  audit DB. Partial dataset accepted — no full re-backfill triggered by
  this fix. Audit-harness Phase 1/2 runs against current 132k.
- **Pool not yet investigated:** d63184… — first 4 bytes `0xd631`
  suggests random hash, not a sentinel. Likely real pool with edge case
  in extraction.
- Bridge backfill plan (Plan B from lore-0185 follow-up) was 374k
  ledgers `62,046,001–62,420,000`. Crashed 27% in.
- Related: 0126 (pool-participants-tracking) introduced
  `lp_positions`. 0179 (lp-asset canonical order) recent LP bug.
