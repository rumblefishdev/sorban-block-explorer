---
id: '0132'
title: 'DB: add missing indexes for planned API query patterns'
type: FEATURE
status: backlog
related_adr: ['0037']
related_tasks: ['0043', '0046', '0050', '0053', '0136', '0167']
tags: [priority-medium, effort-small, layer-db, audit-F21]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F21 (MEDIUM).'
  - date: '2026-04-28'
    status: backlog
    who: stkrolikiewicz
    note: >
      Unblocked: blocker 0136 (surrogate BIGSERIAL ids refactor) is
      `status: superseded` in archive — chain stale. Dropped
      `blocked_by`, moved blocked/ → backlog/. Scope retarget pending
      (current body names `soroban_events`/`operations`; both collapsed
      into `*_appearances` per archived task 0163 + ADR 0037).
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
