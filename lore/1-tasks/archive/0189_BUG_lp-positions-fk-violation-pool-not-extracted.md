---
id: '0189'
title: 'lp_positions FK violation when parent pool not extracted in same ledger'
type: BUG
status: completed
related_adr: ['0041']
related_tasks: ['0126', '0179', '0185', '0193']
tags: ['phase-pools', 'effort-medium', 'priority-high', 'indexer', 'extraction']
links: []
history:
  - date: 2026-05-05
    status: backlog
    who: stkrolikiewicz
    note: 'Crashed bridge backfill (lore-0185 follow-up) at ledger 62148003 with FK violation lp_positions_pool_id_fkey. Spawned for unit-level repro and fix; partial 132k dataset accepted, no full re-backfill needed. Originally created as 0188 — renamed to 0189 due to ID collision with concurrent multi-laptop merge (0188 SEP-1 fetcher).'
  - date: 2026-05-05
    status: active
    who: stkrolikiewicz
    note: 'Promoted to active for unit-level repro + fix. Backfill at 132k partial dataset accepted as audit baseline.'
  - date: 2026-05-05
    status: completed
    who: stkrolikiewicz
    note: >
      Completed via PR #159 (merge commit 749de29). Two-layer fix shipped:
      Layer 3 (extract_liquidity_pools `state` change_type) + sentinel
      placeholder pool with `created_at_ledger=0` marker + sentinel-aware
      UPSERT upgrade. ADR 0041 added (extends 0027/0031/0037, complies with
      0032). 5 new tests (2 unit xdr-parser + 3 integration indexer); all
      207 pre-existing tests still pass; clippy clean. Integration replay
      62148003-62148010 succeeded (8 ledgers indexed without FK violation,
      pool d63184 written with REAL data via Layer 3 — sentinel did NOT
      fire). Audit-harness 15_liquidity_pools.sql + 17_lp_positions.sql
      0 violations; new I6 placeholder count metric. Spawned 0193 (API
      sentinel filter) to backlog. Copilot review addressed in 99cccfa.
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

- [x] Unit test reproducing FK violation on ledger `62148003`, pool
      `\xd63184…`, exercises the same `state`-only-pool path on the new code
      (`crates/xdr-parser/src/state.rs::tests::pool_extracted_from_state_change_type`)
- [x] Root cause identified — extractor asymmetry: `extract_liquidity_pools`
      skipped `state`, `extract_lp_positions` accepted
      `created/updated/restored/removed` for trustlines → orphan
- [x] Fix implemented — Layer 3 (filter loosening) + sentinel placeholder pool
      with `created_at_ledger=0` marker + sentinel-aware UPSERT upgrade
- [x] Unit tests pass after fix (5 new tests: 2 unit in xdr-parser, 3 integration
      in indexer; all 14 indexer integration tests + 193 xdr-parser tests green)
- [x] No regression — pre-existing `15_liquidity_pools.sql` and
      `17_lp_positions.sql` invariants still 0 violations on current DB
- [x] **Docs updated** — ADR 0041 added; `docs/architecture/database-schema/database-schema-overview.md`
      §4.14 documents sentinel placeholder semantics;
      `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md` §5.2
      step 13 documents orphan detection + sentinel emission step

## Implementation Notes

### Files modified

| File                                                                              | Change                                                                                                                                                                                                                                                                      |
| --------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/xdr-parser/examples/decode_pool_ledger.rs`                                | NEW — Step 1 investigation binary; confirmed pool d63184 in ledger 62148003 surfaces as `change_type="state"` with full real data (Lira/liragold pair, fee 30 bps), trustline `change_type="removed"` for account GBBE2CL5…                                                 |
| `crates/xdr-parser/src/state.rs`                                                  | Step 2: `extract_liquidity_pools` filter extended `created/updated/restored` → `created/updated/restored/state`. Plus 2 unit tests: `pool_extracted_from_state_change_type`, `pool_state_only_does_not_promote_to_creation`                                                 |
| `crates/indexer/src/handler/persist/write.rs`                                     | Step 3: `detect_orphan_pool_ids` + `insert_sentinel_pools` helpers. Step 4: 13a UPSERT rewritten with sentinel-aware CASE WHEN logic (existing.created_at_ledger=0 + EXCLUDED.created_at_ledger>0 → upgrade all dimension fields; otherwise preserve existing real values). |
| `crates/indexer/tests/persist_integration.rs`                                     | Step 6: 3 integration tests: `orphan_position_emits_sentinel_pool`, `sentinel_pool_upgraded_on_real_data`, `orphan_detection_skipped_when_pool_in_db`                                                                                                                       |
| `crates/audit-harness/sql/15_liquidity_pools.sql`                                 | Step 5: new `I6 — sentinel placeholder pool count` (informational, not a violation)                                                                                                                                                                                         |
| `lore/2-adrs/0041_lp-positions-orphan-handling-state-filter-and-sentinel-pool.md` | NEW ADR documenting the convention; extends 0027/0031/0037, complies with 0032                                                                                                                                                                                              |
| `docs/architecture/database-schema/database-schema-overview.md` §4.14             | Added paragraph on sentinel placeholder semantics                                                                                                                                                                                                                           |
| `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md` §5.2 step 13  | Expanded with orphan detection + sentinel emission                                                                                                                                                                                                                          |

### Verification results

- 193 xdr-parser unit tests pass (incl. 2 new) + 14 indexer integration tests pass (incl. 3 new)
- `cargo clippy --all-targets --tests -- -D warnings` clean across xdr-parser/indexer/audit-harness
- Integration replay: ran `backfill-runner --start 62148003 --end 62148010` against
  current 132k DB → 8 ledgers indexed without FK violation. Pool d63184 written with
  **real data** (Layer 3 captured `state` snapshot — sentinel did NOT fire). DB now
  at MAX seq=62148010
- Audit-harness `15_liquidity_pools.sql` and `17_lp_positions.sql` invariants: 0
  violations. New `I6` placeholder count: 0 (no orphans encountered in this 8-ledger
  range — pool d63184 was rescued by Layer 3 alone)

## Design Decisions

### From Plan

1. **`created_at_ledger = 0` as sentinel marker**: existing column, no schema migration.
   Stellar pubnet genesis seq is 1; verified 0 existing pools carry value 0 at planning
   time. Detection is single-column `WHERE created_at_ledger = 0`.
2. **Two-layer fix (extractor + persist)**: Layer 3 (filter loosening) covers the
   common case where Stellar Core writes a `state` snapshot of the referenced pool;
   sentinel handles the residual where the pool is not in the current ledger at all.
3. **Sentinel-aware UPSERT upgrade**: 13a `ON CONFLICT DO UPDATE` distinguishes
   `created_at_ledger=0 + EXCLUDED>0` (upgrade) from real-real and real-sentinel
   (no-op) cases via per-column CASE WHEN. `created_at_ledger` itself uses
   `COALESCE(LEAST(NULLIF(...,0), NULLIF(...,0)), 0)` to fold sentinel to NULL for
   LEAST then bring back 0 if both sides sentinel.
4. **Architecture: orphan detection in persist (`write.rs`), not staging**:
   `Staged::prepare()` is sync without DB pool; persist phase already has
   `db_tx: &mut Transaction`. Helpers added at top of `upsert_pools_and_snapshots`
   before the existing 13a INSERT.
5. **Audit invariant `I6` informational, not violation**: sentinel rows are valid
   transient state on partial backfills; should converge to 0 on full from-genesis runs.

### Emerged

6. **Step 1 outcome A confirmed empirically**: pool d63184 in ledger 62148003 has full
   real data via `state` change_type. Layer 3 alone is sufficient for this reproducer.
   Sentinel logic still ships for defense-in-depth on other pools whose `LedgerEntry`
   isn't in the current ledger at all.
7. **`#[allow(clippy::type_complexity)]` on integration test row tuple**: 8-tuple of
   sentinel pool fields exceeded clippy threshold. `#[allow]` was the lightest fix vs
   defining a `type` alias inline (which would have to live outside the test fn).
8. **Single-tuple ORPHAN_LEDGER_SEQ_T1**: T2 declared but never wired into the orphan
   test; removed dead constants.
9. **First-write-wins safety on duplicate state snapshots**: confirmed via existing
   `uq_lp_snapshots_pool_ledger DO NOTHING` (`write.rs:1702`). State views in op_meta
   for the same `(pool_id, ledger_sequence)` carry identical data per Stellar Core
   contract; risk-free under DO NOTHING semantics.

## Notes

- **Backfill state:** 132k ledgers continuous (62,016k → 62,148,002 → now 62,148,010
  after replay) in audit DB. Partial dataset accepted as audit baseline.
- **Pool d63184 reproducer**: Lira/liragold AMM pool, fee 30 bps. First 4 bytes 0xd631
  random hash, not a sentinel. Was a true edge case where the pool was created in a
  pre-window ledger and only appeared as `state` at 62148003.
- Bridge backfill plan (Plan B from lore-0185 follow-up) was 374k
  ledgers `62,046,001–62,420,000`. Crashed 27% in. Bridge backfill not resumed —
  partial 132k accepted as audit baseline.
- Related: 0126 (pool-participants-tracking) introduced
  `lp_positions`. 0179 (lp-asset canonical order) recent LP bug.

## Issues Encountered

- **Plan-mode XDR file path mismatch (FC4DCC04 vs FC4BB25C)**: Initial plan referenced
  hex prefix `FC4DCC04--62148003.xdr.zst` from earlier agent investigation. Verified
  during plan-gap audit that real prefix is `FC4BB25C--62148003.xdr.zst`. Updated
  plan + docstring of `decode_pool_ledger.rs` example. Not a regression — caught
  pre-implementation.
- **Architectural pivot: orphan detection in `write.rs`, not staging**: Initial plan
  put orphan detection in `staging.rs::Staged::prepare()`, which is sync without DB
  pool. Caught in plan-gap audit; moved to `write.rs::upsert_pools_and_snapshots`
  (already has `db_tx: &mut Transaction`). No code lost, just moved.
- **Copilot review confusion on `LEAST(NULLIF(...,0), NULLIF(...,0))` form**: Original
  `created_at_ledger` UPSERT used PG-specific NULL-ignoring `LEAST` semantics
  (PostgreSQL ignores NULLs in LEAST/GREATEST, returns the first non-NULL). Verified
  empirically — integration test passed before and after rewrite. But Copilot's
  misread itself was evidence the form was easy to misunderstand. Rewrote as
  explicit CASE with embedded truth-table comment for portability + clarity. Fix
  shipped in commit `99cccfa`.

**Broken/modified tests:** none. All 5 new tests added (2 unit + 3 integration);
existing 14 indexer integration tests + 193 xdr-parser unit tests still pass.

## Future Work

Spawned to backlog tasks:

- **0193** — API endpoints: filter or annotate sentinel placeholder
  `liquidity_pools` rows. The 5 pool endpoint queries currently surface sentinels
  as garbage data (`native+native, fee=0, ledger=0`). Defer until more sentinels
  appear in production-like partial backfills (current 132k DB has 0 sentinels;
  Layer 3 sufficient for the tested range).

Not yet a separate task — operational, not engineering:

- Full from-genesis backfill — would converge sentinel placeholder count to 0
  permanently. Multi-day operation; out of scope for this fix.
