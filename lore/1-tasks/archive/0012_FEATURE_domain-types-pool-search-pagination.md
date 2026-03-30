---
id: '0012'
title: 'Domain types: liquidity pool, search, pagination, network stats models'
type: FEATURE
status: completed
related_adr: []
related_tasks: []
tags: [priority-high, effort-small, layer-domain]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-27
    status: active
    who: fmazur
    note: 'Task activated'
  - date: 2026-03-27
    status: completed
    who: fmazur
    note: >
      Implemented 11 types in libs/domain/src/index.ts (+108 lines).
      Build, lint, typecheck all pass. Key emerged decision: typed
      PoolAsset instead of raw JsonValue, added totalTrustlines from
      Horizon API review.
---

# Domain types: liquidity pool, search, pagination, network stats models

## Summary

Define the shared TypeScript domain types for liquidity pools, pool snapshots, chart data, network statistics, and search request/response models. These types live in `libs/domain` and are consumed by both `apps/api` and `apps/indexer`. They mirror the PostgreSQL schema for pool tables and the API response contracts for search and network endpoints.

## Status: Completed

## Acceptance Criteria

- [x] `LiquidityPool` type defined with all DDL fields, JSONB asset fields documented
- [x] `LiquidityPoolSnapshot` type defined with all DDL fields, append-only pattern noted
- [x] `PoolChartDataPoint` type defined with createdAt, tvl, volume, feeRevenue
- [x] `PoolChartInterval` union type defined: '1h' | '1d' | '1w'
- [x] `NetworkStats` type defined with all four fields
- [x] `SearchRequest` type defined with q and optional type filter array
- [x] `SearchResultGroup` and `SearchResultItem` types defined
- [x] All types exported from `libs/domain` barrel
- [x] Types compile without errors

## Implementation Notes

All types added to `libs/domain/src/index.ts` (single barrel file, matching existing convention).

**Types added (11):**

- `PoolAsset` — typed asset shape (`asset` + `amount`) replacing raw JSONB
- `LiquidityPool` — current-state pool entity, all DDL fields
- `LiquidityPoolSnapshot` — time-series snapshot, append-only
- `PoolChartInterval` — `'1h' | '1d' | '1w'`
- `PoolChartDataPoint` — chart endpoint response shape
- `NetworkStats` — aggregated explorer-derived metrics
- `SearchEntityType` — discriminated union for searchable entity kinds
- `SearchRequest` — query + optional type filter
- `SearchResultItem` — single search hit
- `SearchResultGroup` — grouped results by entity type

**Reused existing primitives:** `NumericString`, `BigIntString`, `JsonValue`, `readonly` arrays.

## Design Decisions

### From Plan

1. **All types in single barrel file**: Matches existing convention — no separate files per type group.

2. **`NumericString` for financial values**: tvl, totalShares, volume, feeRevenue use `NumericString` to match PostgreSQL `NUMERIC(28,7)`.

3. **`BigIntString` for ledger references**: createdAtLedger, lastUpdatedLedger, ledgerSequence, id fields use `BigIntString` for BIGINT/BIGSERIAL columns.

4. **`readonly` arrays on collections**: reserves, search results, type filters — matches existing pattern (e.g. `SorobanEvent.topics`).

### Emerged

5. **`PoolAsset` interface instead of `JsonValue`**: Task spec used JSONB for asset fields. After reviewing Horizon API docs, the JSONB has a known shape (`{asset: string, amount: NumericString}`). Typed it for safety instead of leaving as `JsonValue`. This is a strictly better choice — no information lost, type safety gained.

6. **`totalTrustlines` added to `LiquidityPool`**: Not in the DDL spec but available from Horizon `total_trustlines` field. Useful for showing pool participant count. Added as `number | null`.

7. **JSDoc on derived fields**: Documented that `tvl`, `volume`, `feeRevenue`, `transactionsPerSecond`, `totalAccounts`, `totalContracts` are explorer-derived — not chain primitives. Added CAP-0038 reference for `feeBps` hardcode.

8. **`SearchEntityType` as discriminated union**: Task spec used raw string for `entityType` in `SearchResultItem`/`SearchResultGroup`. Used `SearchEntityType` union instead for type safety across all search interfaces.

9. **Did NOT add `poolType` field**: Horizon returns `type: "constant_product"` but task DDL doesn't include it. Left for future work to avoid scope creep.

## Issues Encountered

- **`libs/domain/dist/` is gitignored**: Tried to stage build output, git rejected it. Not an issue — dist is correctly excluded, built by consumers.

## Future Work

- Add `poolType` discriminator (`'constant_product' | 'soroban'`) when Soroban DEX pool indexing is implemented
- Consider `'4h'` interval for `PoolChartInterval` if UX requires it

## Notes

- Pool asset fields are JSONB because a pool can pair a classic asset with a Soroban-native token.
- Snapshot metrics (volume, feeRevenue) are explorer-derived, not raw chain values.
- Snapshots are append-only and written in ledger order; they drive the chart endpoint.
- Network stats endpoint should be highly cacheable with short TTLs (5-15 seconds per backend overview).
- Search classifies likely query types and supports exact-match redirect behavior in the frontend.
- Classic Stellar AMM fee is hardcoded at 30 bps per CAP-0038; Soroban DEX pools have custom fee models.
