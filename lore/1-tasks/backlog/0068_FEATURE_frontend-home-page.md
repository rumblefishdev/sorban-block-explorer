---
id: '0068'
title: 'Frontend: Home page'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-medium, layer-frontend-pages]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: Home page

## Summary

Implement the Home page (`/`) as the entry point and chain overview for the Stellar Block Explorer. Provides at-a-glance network state, latest transactions, and latest ledgers with polling-based auto-refresh.

## Status: Backlog

**Current state:** Not started.

## Context

The home page is the fastest way to understand whether the indexer is current and the explorer is healthy. It uses a dashboard summary layout (compact summary cards) followed by latest-activity modules. Polling refreshes summary counts and latest rows without layout jumping.

### API Endpoints Consumed

| Endpoint                     | Purpose                                                                       | Polling     |
| ---------------------------- | ----------------------------------------------------------------------------- | ----------- |
| `GET /network/stats`         | Chain overview: current ledger sequence, TPS, total accounts, total contracts | Yes, 10-15s |
| `GET /transactions?limit=10` | Latest 10 transactions for the activity table                                 | Yes, 10-15s |
| `GET /ledgers?limit=10`      | Latest 10 ledgers for the activity table                                      | Yes, 10-15s |

### Chain Overview Cards

| Card            | Field                   | Source                                    |
| --------------- | ----------------------- | ----------------------------------------- |
| Current Ledger  | Ledger sequence number  | `GET /network/stats` -> `ledger_sequence` |
| TPS             | Transactions per second | `GET /network/stats` -> `tps`             |
| Total Accounts  | Account count           | `GET /network/stats` -> `total_accounts`  |
| Total Contracts | Contract count          | `GET /network/stats` -> `total_contracts` |

### Latest Transactions Table Columns

| Column         | Display                                    | Notes                                   |
| -------------- | ------------------------------------------ | --------------------------------------- |
| Hash           | Truncated, linked to `/transactions/:hash` | Identifier component (task 0062)        |
| Source Account | Truncated, linked to `/accounts/:id`       | Identifier component (task 0062)        |
| Operation Type | Human-readable label                       | e.g., "Payment", "Invoke Contract"      |
| Status         | Badge (success/failed)                     | StatusBadge component (task 0063)       |
| Timestamp      | Relative ("2 min ago")                     | RelativeTimestamp component (task 0063) |

### Latest Ledgers Table Columns

| Column            | Display                        | Notes                                   |
| ----------------- | ------------------------------ | --------------------------------------- |
| Sequence          | Linked to `/ledgers/:sequence` | Identifier component (task 0062)        |
| Closed At         | Relative timestamp             | RelativeTimestamp component (task 0063) |
| Transaction Count | Integer                        | Number of transactions in the ledger    |

## Implementation Plan

### Step 1: Network stats query hook

Create `apps/web/src/pages/home/useNetworkStats.ts`:

- Fetches `GET /network/stats`
- Polling interval: 10-15 seconds
- Stale time: 10-30 seconds

### Step 2: Latest activity query hooks

Create `apps/web/src/pages/home/useLatestTransactions.ts` and `useLatestLedgers.ts`:

- Fetch `GET /transactions?limit=10` and `GET /ledgers?limit=10`
- Polling interval: 10-15 seconds
- No cursor pagination needed (always shows latest)

### Step 3: Chain overview cards section

Create `apps/web/src/pages/home/ChainOverview.tsx`:

- Four compact summary cards: ledger sequence, TPS, accounts, contracts
- Card skeleton loader while loading
- Independent error boundary (failed stats should not collapse activity tables)

### Step 4: Latest transactions section

Create `apps/web/src/pages/home/LatestTransactions.tsx`:

- Table with columns: hash, source account, operation type, status badge, timestamp
- Uses ExplorerTable component (task 0061)
- SectionHeader: "Latest Transactions"
- "View All" link to `/transactions`

### Step 5: Latest ledgers section

Create `apps/web/src/pages/home/LatestLedgers.tsx`:

- Table with columns: sequence, closed_at, tx count
- Uses ExplorerTable component (task 0061)
- SectionHeader: "Latest Ledgers"
- "View All" link to `/ledgers`

### Step 6: Home page composition

Create `apps/web/src/pages/home/HomePage.tsx`:

- Composes: ChainOverview, LatestTransactions, LatestLedgers
- Each section wrapped in SectionErrorBoundary (task 0064)
- Polling indicator visible showing last refresh time (task 0063)
- No layout jump on polling update (stable row heights, no reflow)

## Acceptance Criteria

- [ ] Chain overview cards show: current ledger sequence, TPS, total accounts, total contracts
- [ ] Latest transactions table shows: hash (truncated, linked), source account (truncated, linked), operation type, status badge, timestamp (relative)
- [ ] Latest ledgers table shows: sequence (linked), closed_at (relative), transaction count
- [ ] All three API endpoints polled at 10-15 second intervals
- [ ] Polling updates do not cause layout jump or visual reflow
- [ ] Polling indicator visible showing "Updated Xs ago"
- [ ] Each section has independent error boundary (failed section does not collapse others)
- [ ] Skeleton loaders shown during initial load
- [ ] "View All" links navigate to `/transactions` and `/ledgers`
- [ ] Identifiers are linked to their respective detail pages

## Notes

- The home page is the primary indicator of explorer health and indexer freshness.
- Layout should feel like a dashboard summary, not a dense analytics page.
- The global search bar is already present in the header via the layout shell (task 0059).
