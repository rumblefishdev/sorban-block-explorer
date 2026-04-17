---
id: '0131'
title: 'DB: fix operations partition strategy (transaction_id range useless at scale)'
type: REFACTOR
status: superseded
related_adr: ['0012']
related_tasks: ['0022', '0130', '0140', '0142']
tags:
  [priority-medium, effort-medium, layer-db, audit-F20, superseded-by-adr-0012]
milestone: 1
links:
  - crates/db/migrations/0002_create_operations.sql
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F20 (MEDIUM).'
  - date: '2026-04-17'
    status: superseded
    who: stkrolikiewicz
    by: ['0012']
    note: >
      Superseded by ADR 0012 — operations table repartitioned by RANGE(created_at),
      aligned with soroban_events/soroban_invocations. Decision locked in ADR §"Why
      operations partitions by created_at". Implementation absorbed by 0142 (schema
      migration). See 0140 for audit rationale.
---

# DB: fix operations partition strategy

## Summary

`operations` table is partitioned by `RANGE (transaction_id)` with only `operations_p0`
(0-10M) and `operations_default`. On Stellar mainnet with hundreds of millions of
transactions, virtually all data lands in DEFAULT. Partition pruning never activates for
typical API queries (by source_account, type, etc.).

## Implementation

Options:

1. **Drop partitioning** on operations (simplest — just a regular table).
2. **Switch to time-based partitioning** (add `created_at` column, match events/invocations
   pattern).
3. **Add more range partitions** (quick fix but doesn't solve query pattern mismatch).

Recommendation: Option 1 (drop partitioning) unless there's a specific need for partition
management on operations.

## Acceptance Criteria

- [ ] Operations table has a partition strategy that either (a) benefits queries or (b) is removed
- [ ] Existing data migrated without loss
- [ ] No FK or constraint breakage
