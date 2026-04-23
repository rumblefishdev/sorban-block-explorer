---
id: '0139'
title: 'Align partition Lambda to time-partitioned schema (post-ADR 0027)'
type: BUG
status: completed
related_adr: ['0027']
related_tasks: ['0022', '0131', '0140']
tags: [priority-high, effort-small, layer-infra]
milestone: 1
links:
  - crates/db-partition-mgmt/src/lib.rs
  - crates/db-partition-mgmt/src/main.rs
  - infra/src/lib/stacks/partition-stack.ts
  - crates/db/migrations/0003_transactions_and_operations.sql
history:
  - date: '2026-04-15'
    status: backlog
    who: stkrolikiewicz
    note: >
      Hit in staging deploy on 2026-04-15. operations_default accumulated 1212 rows
      in range 10M-20M because indexer crossed operations_p0 boundary between
      Lambda triggers. Fixed manually: created operations_p1 standalone, moved rows
      from default, attached as partition. Deploy then succeeded.
  - date: '2026-04-15'
    status: active
    who: stkrolikiewicz
    note: 'Activated. Scope: daily cron + Lambda self-heal + alarm fix, all in one task.'
  - date: '2026-04-22'
    status: active
    who: stkrolikiewicz
    note: >
      SCOPE PIVOT. Discovered that ADR 0027 implementation in task 0140
      (commit 998b774) already migrated operations from RANGE (transaction_id)
      to RANGE (created_at) — same monthly scheme as other partitioned tables.
      Original incident cannot recur. Lambda code for operations_pN self-heal
      was dead code against new schema. New scope: delete dead code, add
      operations to the time-partitioned list, remove OperationsRangeHigh
      alarm. See Design Decisions → Emerged.
  - date: '2026-04-23'
    status: completed
    who: stkrolikiewicz
    note: >
      PR #109 merged as commit 8a03b27. 9 files changed (+581/-654 net
      reduction from dead-code strip). 8 unit tests passing incl. new
      regression guard `post_adr_0027_tables_in_time_partitioned_list`.
      Archived task 0131 in same PR (de-facto completed by 0140). Docs
      sweep follow-up tracked in 0155 (augmented, not spawned new).
      Manual staging verification (last AC) deferred to post-deploy;
      ops note in PR body covers indexer-pause prerequisite.
---

# Align partition Lambda to time-partitioned schema

## Summary

Post-ADR 0027 migration moved `operations` (and `transactions`,
`transaction_participants`) from `RANGE (transaction_id)` to
`RANGE (created_at)` monthly. The `db-partition-mgmt` Lambda still carried
range-based code (`operations_pN`, 10M buckets, self-heal logic) that would
fail against the new schema. This task strips the dead code and wires
operations into the existing monthly partition pipeline.

## Context

**Original incident (2026-04-15, old schema):** `operations_default`
accumulated 1212 rows in range 10M-20M because the indexer crossed
`operations_p0` boundary between monthly Lambda triggers. Manually created
`operations_p1` standalone, moved rows, attached.

**Schema change that invalidated it:** `998b774 refactor(lore-0140):
implement ADR 0027 schema from scratch`. Migration
`0003_transactions_and_operations.sql` defines:

```sql
CREATE TABLE operations (
    ...
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),
    ...
) PARTITION BY RANGE (created_at);
```

Lambda code assumed integer range partitioning — would raise a type error
if actually invoked against the new schema (`FOR VALUES FROM (10000000)`
on a TIMESTAMPTZ key).

**Related task 0131** ("Drop partitioning on operations / switch to
time-based") was de-facto completed by 0140's ADR 0027 work. Marked
as completed in this task.

## Implementation

1. **Split `main.rs` → `lib.rs` + thin binary** — enables integration tests
   and keeps handler importable.
2. **Remove dead code** from `lib.rs`:
   - `OPERATIONS_RANGE_SIZE`, `operations_ranges_to_create`, `OperationsResult`
   - `ensure_operations_partitions`, `self_heal_operations_partition`
   - `is_default_overflow_error`, `parse_operations_range_end`
   - Related SQLSTATE constants
3. **Add `operations`, `transactions`, `transaction_participants`** to
   `TIME_PARTITIONED_TABLES` — reuses existing `ensure_time_partitions`.
4. **Regression guard unit test** — assert `operations` is in
   `TIME_PARTITIONED_TABLES`. Catches future re-regression.
5. **Infra `partition-stack.ts`:**
   - `MonthlyPartitionRule` → `DailyPartitionRule` (daily refresh of
     `FuturePartitionCount` metric + partition safety margin)
   - Remove `OperationsRangeHigh` alarm (metric `OperationsRangeUsagePercent`
     no longer published)
   - Extend `FuturePartitions-{table}` alarm loop to all 6 time-partitioned
     tables
6. **Wiki runbook** (`lore/3-wiki/partition-pruning-runbook.md`) — remove
   the "Operations Table (transaction_id range)" section.
7. **Architecture docs** (`docs/architecture/database-schema/database-schema-overview.md`)
   — correct stale `PARTITION BY RANGE (transaction_id)` references.
8. **Archive task 0131** as completed by 0140.

## Acceptance Criteria

- [x] Dead operations_pN code removed from Lambda
- [x] `operations`, `transactions`, `transaction_participants` covered by
      `ensure_time_partitions`
- [x] Daily cron replaces monthly
- [x] `OperationsRangeHigh` alarm removed
- [x] Extended `FuturePartitionCount` alarm coverage to all 6 tables
- [x] Regression unit test asserting operations in time-partitioned list
- [x] `lib.rs`/`main.rs` split for testability
- [x] Wiki runbook updated (operations section removed)
- [x] Architecture docs corrected (`database-schema-overview.md`)
- [x] Task 0131 archived as completed
- [ ] Staging verification: `SELECT * FROM operations_default` returns 0 rows
      (no leftover from old-schema era). **Deferred — manual check post-deploy,
      tracked in PR #109 ops note. Not blocking task completion.**

## Design Decisions

### From Plan

1. **Reuse `ensure_time_partitions`** for operations rather than writing a
   new code path. Consistent with other 3 partitioned tables, no special
   case needed.

2. **Daily cron, not hourly.** Monthly was too sparse; hourly overkill for
   block-explorer write rate. Daily matches growth grain.

### Emerged

3. **Scope pivot from "self-heal + alarm" to "delete dead code"**
   (2026-04-22). Discovered cherry-picked WIP (`59efef4`, 2026-04-17) was
   implementing self-heal for a schema that no longer exists. Schema
   migrated in 0140 (commit 998b774) between task creation (2026-04-15)
   and WIP (2026-04-17), but WIP didn't account for it. Reset branch to
   develop, rewrote from scratch.

4. **Added `transactions` + `transaction_participants` to the time-partitioned
   list** (not just `operations`). Plan mentioned only operations, but
   migration 0003 makes all three `PARTITION BY RANGE (created_at)`. If we
   skip them, their monthly partitions never get created — Lambda would
   silently ignore the tables that need it most (transactions is the busiest
   write path).

5. **Removed `OperationsRangeHigh` alarm entirely** rather than repurposing.
   Metric `OperationsRangeUsagePercent` is no longer published; keeping the
   alarm would alarm on missing data permanently. Clean deletion.

6. **No integration test for time-partitions.** `ensure_time_partitions` is
   unchanged code with existing unit-test coverage. Integration test would
   add container overhead (~$30/month CI Postgres) for no new behavior
   coverage. Regression risk for "operations missing from table list" is
   covered by the unit test `operations_in_time_partitioned_tables`.

7. **Task 0131 archived as completed by 0140** rather than closed as
   duplicate. 0140 (ADR 0027 schema rebuild) wasn't written as a fix for
   0131, but its scope subsumed it. `completed` with cross-reference is
   more honest than `superseded`.

## Issues Encountered

- **WIP commit `59efef4` targeted wrong schema** — written 2026-04-17,
  2 days after 0140 merged (2026-04-15 for `89f4335`, 2026-04-16 for
  `998b774` merge PR #98). Either WIP author missed the schema change,
  or parked it without noting why. Task file gave no hint; only discovered
  by cross-referencing migration `0003` with Lambda code during review.

- **Architecture docs lagged schema** — `docs/architecture/database-schema/database-schema-overview.md`
  still claimed `PARTITION BY RANGE (transaction_id)` on lines 189, 202, 513. Fixed as part of this task.

## Manual Runbook (post-deploy verification)

If staging/prod `operations_default` has stale rows from the old-schema era
(transaction_id-based partitioning), they'd sit there invisibly. Check and
clean:

```sql
SELECT COUNT(*) FROM operations_default;
-- expect 0 after ADR 0027 migration ran; if >0, investigate before next deploy.
```

If old-schema `operations_p0` / `operations_p1` tables still exist as
orphans (detached from parent after 0140 migration recreated `operations`):

```sql
SELECT relname FROM pg_class WHERE relname LIKE 'operations_p%';
-- any results → orphaned, drop after verifying they are detached:
DROP TABLE operations_p0;  -- etc.
```
