---
id: '0130'
title: 'DB: create historical partitions for backfill (2023-11 through 2026-03)'
type: BUG
status: backlog
related_adr: ['0012']
related_tasks: ['0022', '0117', '0140', '0142']
blocked_by: ['0142']
tags:
  [priority-high, effort-small, layer-db, audit-F19, pending-adr-0012-rewrite]
milestone: 1
links:
  - crates/db/migrations/0004_create_soroban_activity_tables.sql
  - crates/db/migrations/0006_create_nfts_pools_snapshots.sql
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F19 (MEDIUM). Must be done BEFORE historical backfill (backfill (backfill-bench, task 0117)).'
  - date: '2026-04-17'
    status: backlog
    who: stkrolikiewicz
    note: >
      Audit per task 0140 — ADR 0012 supersedes the underlying schema/flow
      patterns referenced in body. Blocked by 0142 (schema migration). Body
      must be re-read against ADR 0012 before implementing.
---

> **⚠ Post-ADR 0012 re-read required (audit 2026-04-17, [task 0140](../active/0140_DOCS_audit-lore-tasks-adr-0011-0012.md)):**
> Body below references pre-ADR-0012 patterns (schema / flow / JSONB / upsert / `transaction_id` partitioning). [ADR 0012](../../2-adrs/0012_zero-upsert-schema-full-fk-graph.md) supersedes with zero-upsert history tables, activity projections, S3 offload, and `created_at` partitioning. Blocked by 0142 (schema migration) — do not implement until migration lands and this task is re-aligned.

---

# DB: create historical partitions for backfill (2023-11 through 2026-03)

## Summary

Only Apr-Jun 2026 partitions exist for `soroban_events`, `soroban_invocations`, and
`liquidity_pool_snapshots`. Historical backfill (backfill (backfill-bench, task 0117)) will insert data from Nov 2023
through Mar 2026 — all of which lands in the DEFAULT partition, defeating the purpose of
partitioning and creating major query performance issues.

Splitting a populated DEFAULT partition later requires exclusive locks and data migration.
This MUST be done before backfill runs.

## Context

Task 0022 (partition management automation) is complete — the Lambda creates future
partitions 3+ months ahead. But it does not retroactively create historical partitions.

## Implementation

1. New migration: create monthly partitions from 2023-11 through 2026-03 (29 months) for
   all three partitioned tables.
2. Verify DEFAULT partition is empty before running (no data loss risk).
3. Run migration before historical backfill backfill (backfill-bench, task 0117).

## Acceptance Criteria

- [ ] Monthly partitions exist for 2023-11 through 2026-03 on soroban_events
- [ ] Monthly partitions exist for 2023-11 through 2026-03 on soroban_invocations
- [ ] Monthly partitions exist for 2023-11 through 2026-03 on liquidity_pool_snapshots
- [ ] DEFAULT partitions remain empty after backfill
- [ ] Migration is idempotent (safe to re-run)
