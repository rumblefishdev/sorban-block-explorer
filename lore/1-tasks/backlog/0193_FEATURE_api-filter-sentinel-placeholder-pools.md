---
id: '0193'
title: 'API endpoints: filter or annotate sentinel placeholder liquidity_pools rows'
type: FEATURE
status: backlog
related_adr: ['0041']
related_tasks: ['0189']
tags: ['phase-future', 'effort-small', 'priority-low', 'api', 'liquidity-pools']
links:
  - lore/2-adrs/0041_lp-positions-orphan-handling-state-filter-and-sentinel-pool.md
  - docs/architecture/database-schema/endpoint-queries/18_get_liquidity_pools_list.sql
  - docs/architecture/database-schema/endpoint-queries/19_get_liquidity_pools_by_id.sql
  - docs/architecture/database-schema/endpoint-queries/20_get_liquidity_pools_transactions.sql
  - docs/architecture/database-schema/endpoint-queries/21_get_liquidity_pools_chart.sql
  - docs/architecture/database-schema/endpoint-queries/23_get_liquidity_pools_participants.sql
history:
  - date: 2026-05-05
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from 0189 future work. Sentinel placeholder pools (`created_at_ledger=0`) currently surface as garbage rows ("native+native, fee=0, ledger 0") in pool endpoints; needs filter or `is_partial_data` flag in API response.'
---

# API endpoints: filter or annotate sentinel placeholder liquidity_pools rows

## Summary

Per [ADR 0041](../../2-adrs/0041_lp-positions-orphan-handling-state-filter-and-sentinel-pool.md),
the indexer can emit sentinel placeholder rows in `liquidity_pools` for pools
referenced by `lp_positions` but not observable in the current ledger (typical
for partial / mid-stream backfills). The marker is `created_at_ledger = 0`.

5 pool endpoint queries currently read `liquidity_pools` directly without a
sentinel filter — they surface placeholder rows as garbage data
(`asset_a_type=NATIVE, asset_a_code=NULL, asset_b_type=NATIVE,
asset_b_code=NULL, fee_bps=0, fee_percent=0, created_at_ledger=0`):

- `docs/architecture/database-schema/endpoint-queries/18_get_liquidity_pools_list.sql`
- `19_get_liquidity_pools_by_id.sql`
- `20_get_liquidity_pools_transactions.sql`
- `21_get_liquidity_pools_chart.sql`
- `23_get_liquidity_pools_participants.sql`

This task adds either a filter (`WHERE created_at_ledger > 0`) or a
`is_partial_data` flag in the API response so consumers can render the
placeholder state explicitly.

## Context

Production indexer (Galexie / Lambda live) starts from genesis and never emits
sentinels. Sentinel rows only appear in partial backfills used for audit and
sample-based testing. Until full from-genesis backfill is complete, the
sample-based DB will carry some placeholders.

## Implementation Plan

### Step 1: Decide approach

Two options — pick one:

- **A. Hard filter**: add `WHERE created_at_ledger > 0` to all 5 endpoint
  queries + update DTOs as needed. Pros: simple, hides partial-data noise.
  Cons: lose visibility into partial-backfill coverage at API surface.

- **B. Annotate**: include sentinel pools in responses with explicit
  `is_partial_data: true` flag (computed from `created_at_ledger = 0`).
  Pros: transparent. Cons: every pool DTO needs a flag field; consumer must
  handle.

Frontend lead input recommended.

### Step 2: Implement chosen approach

- Update queries (in `crates/api/src/liquidity_pools/queries.rs` if mirrored
  from `docs/architecture/database-schema/endpoint-queries/`)
- Update DTOs (`crates/api/src/liquidity_pools/dto.rs`) if option B
- Update `docs/architecture/database-schema/endpoint-queries/*.sql` per
  [ADR 0032](../../2-adrs/0032_docs-architecture-evergreen-maintenance.md)

### Step 3: Tests

- Integration tests in `crates/api/tests/` covering sentinel pool: hidden
  (option A) or flagged (option B)

## Acceptance Criteria

- [ ] Decision recorded — A vs B
- [ ] All 5 endpoint queries updated
- [ ] DTOs updated if option B
- [ ] Integration tests cover sentinel pool behavior
- [ ] **Docs updated** per ADR 0032 — endpoint-queries/\*.sql + relevant
      backend overview sections
- [ ] No regression on real pool queries

## Notes

- 132k partial backfill DB currently has 0 sentinel pools (Layer 3 captured
  all in tested ledger range), but the API filter is still needed
  defensively for future partial backfills and the long-tail edge case where
  a pool was created in a pre-window ledger and never touched again as
  `state` either.
- Detection criterion is single-column: `WHERE created_at_ledger = 0`. No
  additional schema needed.
