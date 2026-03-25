---
id: '0057'
title: 'Frontend: Liquidity Pools list and detail pages'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-medium, effort-large, layer-frontend-pages]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: Liquidity Pools list and detail pages

## Summary

Implement the Liquidity Pools list page (`/liquidity-pools`) and detail page (`/liquidity-pools/:id`). Includes pool summaries, time-series charts (TVL, volume, fee revenue), pool participants, and transaction history with visual distinction between trades and liquidity management.

## Status: Backlog

**Current state:** Not started.

## Context

Liquidity pool pages combine factual current-state data with historical time-series visualizations. The detail page is one of the most visually complex pages in the explorer due to charts. The summary area anchors the page before users move into charts and participant data.

### API Endpoints Consumed

| Endpoint                                | Query Params                                           | Purpose                                             |
| --------------------------------------- | ------------------------------------------------------ | --------------------------------------------------- |
| `GET /liquidity-pools`                  | `limit`, `cursor`, `filter[assets]`, `filter[min_tvl]` | Paginated pool list with asset pair and TVL filters |
| `GET /liquidity-pools/:id`              | none                                                   | Pool detail: asset pair, fee, reserves, shares, TVL |
| `GET /liquidity-pools/:id/transactions` | `limit`, `cursor`                                      | Deposits, withdrawals, and trades for this pool     |
| `GET /liquidity-pools/:id/chart`        | `interval` (1h\|1d\|1w), `from`, `to`                  | Time-series data: TVL, volume, fee revenue          |

### Pool List Table Columns

| Column       | Display                                     | Notes                                                     |
| ------------ | ------------------------------------------- | --------------------------------------------------------- |
| Pool ID      | Truncated, linked to `/liquidity-pools/:id` | IdentifierDisplay (task 0042)                             |
| Asset Pair   | e.g., "XLM / USDC"                          | Both assets displayed, linked to token pages if available |
| Total Shares | Formatted number                            | Total pool shares                                         |
| Reserves     | Per-asset reserves                          | e.g., "1,250,000 XLM / 500,000 USDC"                      |
| Fee %        | Percentage                                  | Pool fee percentage                                       |

### Pool List Filters

| Filter      | Type         | Notes                                           |
| ----------- | ------------ | ----------------------------------------------- |
| Assets      | Text input   | Filters by `filter[assets]` (asset pair search) |
| Minimum TVL | Number input | Filters by `filter[min_tvl]`                    |

- Filters reflected in URL, filter change resets cursor

### Pool Detail Summary Fields

| Field          | Display            | Notes                                             |
| -------------- | ------------------ | ------------------------------------------------- |
| Pool ID        | Full, copyable     | IdentifierWithCopy (task 0042). Prominent at top. |
| Asset Pair     | e.g., "XLM / USDC" | Both assets displayed and linked                  |
| Fee Percentage | Percentage         | Pool fee                                          |
| Total Shares   | Formatted number   | Total shares in pool                              |
| Reserves       | Per-asset reserves | Both asset amounts with formatting                |

### Charts Section

| Chart            | Data                            | Notes           |
| ---------------- | ------------------------------- | --------------- |
| TVL Over Time    | Time-series from chart endpoint | Line/area chart |
| Volume Over Time | Time-series from chart endpoint | Line/area chart |
| Fee Revenue      | Time-series from chart endpoint | Line/area chart |

- Interval selector: 1h, 1d, 1w (triggers re-fetch with new interval param)
- Date range selector (optional)
- Responsive: readable on small screens
- Lazy-loaded: only fetch chart data when section is visible (task 0045)

### Pool Participants Table (if API supports)

| Column   | Display                              | Notes                         |
| -------- | ------------------------------------ | ----------------------------- |
| Provider | Truncated, linked to `/accounts/:id` | IdentifierDisplay (task 0042) |
| Share    | Formatted number/percentage          | Provider's share of the pool  |

### Pool Transactions Table Columns

| Column    | Display                                    | Notes                                                   |
| --------- | ------------------------------------------ | ------------------------------------------------------- |
| Hash      | Truncated, linked to `/transactions/:hash` | IdentifierDisplay (task 0042)                           |
| Type      | Badge/label                                | Deposit, Withdrawal, or Trade -- visually distinguished |
| Amount    | Formatted                                  | Transaction amount(s)                                   |
| Account   | Truncated, linked to `/accounts/:id`       | IdentifierDisplay (task 0042)                           |
| Timestamp | Relative                                   | RelativeTimestamp (task 0043)                           |

- Deposits, withdrawals, and trades visually distinguished (different labels/badges)
- Paginated with cursor-based pagination

## Implementation Plan

### Step 1: Pool list query hook and page

Create `apps/web/src/pages/liquidity-pools/usePoolsList.ts` and `LiquidityPoolsListPage.tsx`:

- Fetches `GET /liquidity-pools` with filters and cursor
- Filter controls: assets text input, minimum TVL number input
- Table with columns: pool ID, asset pair, total shares, reserves, fee %

### Step 2: Pool detail query hooks

Create `apps/web/src/pages/pool-detail/`:

- `usePoolDetail.ts`: fetches `GET /liquidity-pools/:id`, stale time 5 minutes
- `usePoolTransactions.ts`: fetches `GET /liquidity-pools/:id/transactions` with cursor
- `usePoolChart.ts`: fetches `GET /liquidity-pools/:id/chart` with interval, from, to params

### Step 3: Pool summary section

Create `apps/web/src/pages/pool-detail/PoolSummary.tsx`:

- Renders: pool ID (full, copyable), asset pair, fee %, total shares, reserves
- Anchors page at top before charts

### Step 4: Charts section

Create `apps/web/src/pages/pool-detail/PoolCharts.tsx`:

- Uses TimeSeriesChart component (task 0045)
- Three charts: TVL, volume, fee revenue
- Interval selector: 1h, 1d, 1w
- Lazy-loaded via LazySection (task 0045)

### Step 5: Pool participants section

Create `apps/web/src/pages/pool-detail/PoolParticipants.tsx`:

- Table of providers and their share (if API supports)
- Graceful empty state if data not available

### Step 6: Pool transactions section

Create `apps/web/src/pages/pool-detail/PoolTransactions.tsx`:

- Paginated table: hash, type (deposit/withdrawal/trade), amount, account, timestamp
- Type visually distinguished with labels or badges
- SectionHeader: "Transactions"

### Step 7: Page composition

Create `apps/web/src/pages/pool-detail/LiquidityPoolDetailPage.tsx`:

- Composes: PoolSummary, PoolCharts, PoolParticipants, PoolTransactions
- Each section in SectionErrorBoundary (task 0044)
- 404 state: "Liquidity pool not found"

## Acceptance Criteria

- [ ] Pool list columns: pool ID (truncated, linked), asset pair, total shares, reserves, fee %
- [ ] List filters: assets, minimum TVL. Reflected in URL.
- [ ] Detail summary: pool ID (full, copyable), asset pair, fee %, shares, reserves
- [ ] Summary anchors page at top before charts
- [ ] Charts: TVL, volume, fee revenue over time with interval selector (1h/1d/1w)
- [ ] Charts responsive on small screens
- [ ] Charts lazy-loaded (only fetched when visible)
- [ ] Pool transactions: visually distinguish deposits, withdrawals, and trades
- [ ] Transactions paginated with cursor-based pagination
- [ ] Reserves/TVL formatting consistent across app
- [ ] 404 state: "Liquidity pool not found"
- [ ] Loading skeleton and error states per section

## Notes

- This is the largest effort page task due to the combination of summary data, charts, and transaction history.
- Chart rendering depends on visualization primitives from task 0045.
- The chart endpoint supports interval-based aggregation; the frontend should pass interval params and let the backend aggregate.
- Pool participants section may not be available initially if the API does not support it. Handle gracefully.
