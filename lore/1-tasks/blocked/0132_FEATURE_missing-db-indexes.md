---
id: '0132'
title: 'DB: add missing indexes for planned API query patterns'
type: FEATURE
status: blocked
related_adr: ['0012']
related_tasks: ['0043', '0046', '0050', '0053', '0136', '0140', '0142']
blocked_by: ['0136', '0142']
tags:
  [priority-medium, effort-small, layer-db, audit-F21, pending-adr-0012-rewrite]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F21 (MEDIUM).'
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

# DB: add missing indexes for planned API query patterns

## Summary

Several planned API query patterns lack supporting indexes:

1. `soroban_events` — no composite index on `(contract_id, event_type, created_at)` for
   type-filtered event queries.
2. `operations` — no index on `type` column for operation-type filtering.

## Implementation

New migration with:

```sql
CREATE INDEX idx_events_contract_type
  ON soroban_events (contract_id, event_type, created_at DESC);

CREATE INDEX idx_operations_type
  ON operations (type);
```

## Acceptance Criteria

- [ ] Events filterable by (contract_id, event_type) with index scan
- [ ] Operations filterable by type with index scan
- [ ] No regression on existing query performance
