---
id: '0126'
title: 'LP: pool participants and share tracking'
type: FEATURE
status: blocked
related_adr: ['0012']
related_tasks: ['0052', '0077', '0136', '0140', '0142']
blocked_by: ['0136', '0142']
tags:
  [
    priority-low,
    effort-medium,
    layer-indexer,
    layer-db,
    audit-gap,
    pending-adr-0012-rewrite,
  ]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — tech design specifies pool participants table on LP detail page but no schema exists.'
  - date: '2026-04-17'
    status: blocked
    who: stkrolikiewicz
    note: >
      Audit per task 0140 — ADR 0012 supersedes the underlying schema/flow
      patterns referenced in body. Blocked by 0142 (schema migration). Body
      must be re-read against ADR 0012 before implementing.
---

> **⚠ Post-ADR 0012 re-read required (audit 2026-04-17, [task 0140](../active/0140_DOCS_audit-lore-tasks-adr-0011-0012.md)):**
> Body below references pre-ADR-0012 patterns (schema / flow / JSONB / upsert / `transaction_id` partitioning). [ADR 0012](../../2-adrs/0012_zero-upsert-schema-full-fk-graph.md) supersedes with zero-upsert history tables, activity projections, S3 offload, and `created_at` partitioning. Blocked by 0142 (schema migration) — do not implement until migration lands and this task is re-aligned.

---

# LP: pool participants and share tracking

## Summary

The technical design specifies a "Pool participants" section on the LP detail page showing
liquidity providers and their share. No per-provider tracking exists in the current schema.

## Implementation

1. Create `liquidity_pool_participants` table (pool_id, account_id, shares, last_updated).
2. Track pool share changes from `LedgerEntryChanges` — trustline entries for pool shares.
3. Alternatively, derive from `soroban_events` or `soroban_invocations` for deposit/withdraw
   activity.

## Acceptance Criteria

- [ ] Per-provider pool shares trackable
- [ ] API endpoint returns participants for a given pool
- [ ] Shares updated on deposit/withdrawal events
