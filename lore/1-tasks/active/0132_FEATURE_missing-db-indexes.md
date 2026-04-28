---
id: '0132'
title: 'DB: add missing indexes for planned API query patterns'
type: FEATURE
status: active
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
  - date: '2026-04-28'
    status: backlog
    who: stkrolikiewicz
    note: >
      Rescoped — replaced the two stale indexes with the three concrete
      gaps surfaced by 0167's per-endpoint EXPLAIN audit (idx_tx_keyset,
      idx_nfts_collection_trgm, idx_pools_created_at_ledger). All three
      flagged inline as `INDEX GAP:` comments in
      `docs/architecture/database-schema/endpoint-queries/`. Confirmed
      not a backfill blocker (CONCURRENTLY, post-restore).
  - date: '2026-04-28'
    status: backlog
    who: fmazur
    note: >
      Added two more indexes from Copilot review on PR 136 (task 0172
      / E02 Statement B variant 2): the contract-leading partial
      indexes on `soroban_invocations_appearances` and
      `soroban_events_appearances` don't align with Statement B's
      `(created_at, transaction_id)` keyset, forcing a sort step on
      popular-contract queries at mainnet scale. Same migration,
      same `CONCURRENTLY` rule. Inline INDEX GAP comment added in
      02_get_transactions_list.sql header.
  - date: '2026-04-28'
    status: active
    who: stkrolikiewicz
    note: 'Promoted to active via /promote-task — implementation track. Body covers 5 indexes after fmazur addition for E02 Statement B variant 2.'
---

# DB: add missing indexes for planned API query patterns

## Summary

Five concrete index gaps surfaced by per-endpoint EXPLAIN audits ([task
0167](../archive/0167_FEATURE_endpoint-sql-query-reference-set.md) +
PR 136 review on E02 Statement B variant 2). All flagged inline as
`INDEX GAP:` comments inside
[`docs/architecture/database-schema/endpoint-queries/`](../../../docs/architecture/database-schema/endpoint-queries/);
this task wraps them up into a single migration.

## Implementation

New migration with `CREATE INDEX CONCURRENTLY` (so it can run after the
backfill restore without holding an AccessExclusiveLock):

```sql
-- E2 GET /transactions — no-filter keyset on (created_at DESC, id DESC).
-- Without it, the planner falls back to per-partition seq + sort.
-- See 02_get_transactions_list.sql:33.
CREATE INDEX CONCURRENTLY idx_tx_keyset
  ON transactions (created_at DESC, id DESC);

-- E15 GET /nfts — collection_name filter is currently exact `=` against
-- a btree; the endpoint contract wants ILIKE. Trigram GIN unblocks ILIKE.
-- See 15_get_nfts_list.sql:30.
CREATE INDEX CONCURRENTLY idx_nfts_collection_trgm
  ON nfts USING gin (collection_name gin_trgm_ops);

-- E18 GET /liquidity-pools — keyset on (created_at_ledger DESC, pool_id DESC).
-- liquidity_pools is small enough today that a heap scan + sort is
-- tolerable, but the cost grows linearly. Pre-emptively index.
-- See 18_get_liquidity_pools_list.sql:31.
CREATE INDEX CONCURRENTLY idx_pools_created_at_ledger
  ON liquidity_pools (created_at_ledger DESC, pool_id DESC);

-- E2 Statement B (variant 2) — broad-match contract filter UNIONs three
-- appearance tables and keyset-orders the result by
-- (created_at DESC, transaction_id DESC). The two below align the
-- soroban_invocations_appearances and soroban_events_appearances
-- contract-leading indexes with that cursor; the existing
-- idx_sia_contract_ledger / idx_sea_contract_ledger lead with
-- ledger_sequence and don't carry the keyset shape. On rare
-- contracts the planner falls back to the composite PK and works
-- (sub-ms in 100-ledger sample); on a popular contract with millions
-- of rows mainnet-side it forces a sort step. Owner alternative:
-- switch the UNION branches to keyset on `ledger_sequence` and skip
-- these indexes (uses existing indexes natively but introduces a
-- second cursor flavor in the API).
-- See 02_get_transactions_list.sql header (INDEX GAP — Statement B).
CREATE INDEX CONCURRENTLY idx_sia_contract_keyset
  ON soroban_invocations_appearances
     (contract_id, created_at DESC, transaction_id DESC);

CREATE INDEX CONCURRENTLY idx_sea_contract_keyset
  ON soroban_events_appearances
     (contract_id, created_at DESC, transaction_id DESC);
```

## Acceptance Criteria

- [ ] `idx_tx_keyset` exists and is used by E2 in the no-filter case
      (`EXPLAIN` shows index scan, not partition + sort)
- [ ] `idx_nfts_collection_trgm` exists and supports ILIKE on
      `collection_name` (`EXPLAIN` shows bitmap heap scan via the GIN)
- [ ] `idx_pools_created_at_ledger` exists and is used by E18 keyset
      ordering
- [ ] `idx_sia_contract_keyset` exists and is used by E2 Statement B's
      `soroban_invocations_appearances` UNION branch (`EXPLAIN` shows
      index-only scan with the (created_at, transaction_id) keyset
      walking the index, not a sort step) — OR owner accepts the
      ledger_sequence-keyset alternative and these two indexes are
      skipped; document the choice in the migration commit.
- [ ] `idx_sea_contract_keyset` exists and is used by E2 Statement B's
      `soroban_events_appearances` UNION branch (same plan check) —
      same alternative caveat as above.
- [ ] [ADR 0037 §4](../../2-adrs/0037_current-schema-snapshot.md) updated
      with the new indexes (per ADR 0032 evergreen-docs rule)
- [ ] No regression on `EXPLAIN ANALYZE` for the other E1–E23 queries

## Notes

- **Does not block backfill T0.** All five use `CONCURRENTLY` and can be
  added post-restore on staging; the backfill execution plan
  ([wiki](../../3-wiki/backfill-execution-plan.md)) lists this as a
  post-cutover step.
- **Original body is obsolete.** It targeted `soroban_events(contract_id,
event_type, created_at)` and `operations(type)` — both base tables were
  collapsed into `*_appearances` by archived task 0163, and the queries
  that would have used them no longer exist in the endpoint set.
- **Last two indexes have an alternative.** `idx_sia_contract_keyset` /
  `idx_sea_contract_keyset` solve the perf gap by aligning the index
  with E2 Statement B's cursor. Owner can instead change the SQL to
  keyset on `ledger_sequence` in the two UNION branches (already covered
  by `idx_sia_contract_ledger` / `idx_sea_contract_ledger`), at the
  cost of a second cursor flavor in the API. Both fixes equivalent
  for plan quality; pick one before backfill.
