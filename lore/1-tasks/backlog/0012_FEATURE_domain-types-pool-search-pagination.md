---
id: '0012'
title: 'Domain types: liquidity pool, search, pagination, network stats models'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0012']
tags: [priority-high, effort-small, layer-domain]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Domain types: liquidity pool, search, pagination, network stats models

## Summary

Define the shared TypeScript domain types for liquidity pools, pool snapshots, chart data, network statistics, and search request/response models. These types live in `libs/domain` and are consumed by both `apps/api` and `apps/indexer`. They mirror the PostgreSQL schema for pool tables and the API response contracts for search and network endpoints.

## Status: Backlog

**Current state:** Not started. Depends on DB schema task 0012.

## Context

Liquidity pools, search, and network stats are the remaining explorer entities that need shared domain types. Pools have both current-state and time-series snapshot tables. Search and network stats are API-facing models that do not directly map to a single table but aggregate across the schema.

### LiquidityPool fields (from DDL)

| Field             | DB Type                 | Notes                                               |
| ----------------- | ----------------------- | --------------------------------------------------- |
| poolId            | VARCHAR(64) PRIMARY KEY |                                                     |
| assetA            | JSONB NOT NULL          | Pool assets may span classic and Soroban identities |
| assetB            | JSONB NOT NULL          | Pool assets may span classic and Soroban identities |
| feeBps            | INT nullable            | Fee in basis points                                 |
| reserves          | JSONB NOT NULL          |                                                     |
| totalShares       | NUMERIC(28,7) nullable  |                                                     |
| tvl               | NUMERIC(28,7) nullable  | Total value locked                                  |
| createdAtLedger   | BIGINT FK nullable      | References ledgers(sequence)                        |
| lastUpdatedLedger | BIGINT FK nullable      | References ledgers(sequence)                        |

Index: `idx_last_updated (last_updated_ledger DESC)`.

Assets are JSONB because pool assets may span classic and Soroban-native identities.

### LiquidityPoolSnapshot fields (from DDL)

| Field          | DB Type                | Notes                               |
| -------------- | ---------------------- | ----------------------------------- |
| id             | BIGSERIAL PRIMARY KEY  |                                     |
| poolId         | VARCHAR(64) FK CASCADE | References liquidity_pools(pool_id) |
| ledgerSequence | BIGINT NOT NULL        |                                     |
| createdAt      | TIMESTAMPTZ NOT NULL   |                                     |
| reserves       | JSONB NOT NULL         |                                     |
| totalShares    | NUMERIC(28,7) nullable |                                     |
| tvl            | NUMERIC(28,7) nullable |                                     |
| volume         | NUMERIC(28,7) nullable |                                     |
| feeRevenue     | NUMERIC(28,7) nullable |                                     |

Monthly partitioned by `created_at`. Append-only, written in ledger order. Metrics (`volume`, `feeRevenue`) are explorer-derived measures, not chain primitives.

### PoolChartDataPoint type

```typescript
{
  createdAt: Date;
  tvl: string;
  volume: string;
  feeRevenue: string;
}
```

Chart query parameters: `interval` ('1h' | '1d' | '1w'), `from: Date`, `to: Date`.

Served by `GET /liquidity-pools/:id/chart`.

### NetworkStats type

```typescript
{
  currentLedgerSequence: bigint;
  transactionsPerSecond: number;
  totalAccounts: number;
  totalContracts: number;
}
```

Served by `GET /network/stats`. Should be small, fast, and cacheable with short TTLs.

### SearchRequest type

```typescript
{
  q: string;
  type?: ('transaction' | 'contract' | 'token' | 'account' | 'nft' | 'pool')[];
}
```

### SearchResultGroup type

```typescript
{
  entityType: string;
  count: number;
  results: SearchResultItem[];
}
```

### SearchResultItem type

```typescript
{
  identifier: string;
  entityType: string;
  context: string;
}
```

Search uses prefix/exact matching on hashes, account IDs, contract IDs, asset codes, pool IDs, and NFT identifiers. Full-text search on metadata via tsvector/tsquery and GIN indexes.

## Implementation Plan

### Step 1: Define LiquidityPool domain type

Create `LiquidityPool` type with all DDL fields. Document that asset fields are JSONB because they span classic and Soroban identities.

### Step 2: Define LiquidityPoolSnapshot domain type

Create `LiquidityPoolSnapshot` type with all DDL fields. Note append-only write pattern and monthly partitioning.

### Step 3: Define PoolChartDataPoint and chart query types

Create `PoolChartDataPoint` type and `PoolChartInterval` union ('1h' | '1d' | '1w'). Define chart query parameters type with interval, from, to.

### Step 4: Define NetworkStats type

Create `NetworkStats` type with currentLedgerSequence, transactionsPerSecond, totalAccounts, totalContracts.

### Step 5: Define Search types

Create `SearchRequest`, `SearchResultGroup`, and `SearchResultItem` types. Define the `SearchEntityType` union for the optional type filter.

### Step 6: Export and verify

Export all types from `libs/domain` barrel file. Verify compilation and field alignment with DDL and API contracts.

## Acceptance Criteria

- [ ] `LiquidityPool` type defined with all DDL fields, JSONB asset fields documented
- [ ] `LiquidityPoolSnapshot` type defined with all DDL fields, append-only pattern noted
- [ ] `PoolChartDataPoint` type defined with createdAt, tvl, volume, feeRevenue
- [ ] `PoolChartInterval` union type defined: '1h' | '1d' | '1w'
- [ ] `NetworkStats` type defined with all four fields
- [ ] `SearchRequest` type defined with q and optional type filter array
- [ ] `SearchResultGroup` and `SearchResultItem` types defined
- [ ] All types exported from `libs/domain` barrel
- [ ] Types compile without errors

## Notes

- Pool asset fields are JSONB because a pool can pair a classic asset with a Soroban-native token.
- Snapshot metrics (volume, feeRevenue) are explorer-derived, not raw chain values.
- Snapshots are append-only and written in ledger order; they drive the chart endpoint.
- Network stats endpoint should be highly cacheable with short TTLs (5-15 seconds per backend overview).
- Search classifies likely query types and supports exact-match redirect behavior in the frontend.
