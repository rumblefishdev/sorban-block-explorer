---
id: '0069'
title: 'Frontend: Transactions list page'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-medium, effort-medium, layer-frontend-pages]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: Transactions list page

## Summary

Implement the Transactions list page (`/transactions`) showing a paginated, filterable table of all indexed transactions, sorted most-recent-first by default.

## Status: Backlog

**Current state:** Not started.

## Context

This page is the primary browsing surface for all network transaction activity. It supports rapid scanning with filters, cursor-based pagination, and URL-synced state. The table must remain usable on large datasets without assuming total counts.

### API Endpoint Consumed

| Endpoint            | Query Params                                                                                 | Purpose                                |
| ------------------- | -------------------------------------------------------------------------------------------- | -------------------------------------- |
| `GET /transactions` | `limit`, `cursor`, `filter[source_account]`, `filter[contract_id]`, `filter[operation_type]` | Paginated, filterable transaction list |

### Table Columns

| Column          | Display                                    | Notes                                                |
| --------------- | ------------------------------------------ | ---------------------------------------------------- |
| Hash            | Truncated, linked to `/transactions/:hash` | Identifier component (task 0062)                     |
| Ledger Sequence | Linked to `/ledgers/:sequence`             | Identifier component (task 0062)                     |
| Source Account  | Truncated, linked to `/accounts/:id`       | Identifier component (task 0062)                     |
| Operation Type  | Human-readable label                       | e.g., "Payment", "Create Account", "Invoke Contract" |
| Status          | Badge (success/failed)                     | StatusBadge component (task 0063)                    |
| Fee             | Display value                              | XLM amount                                           |
| Timestamp       | Relative ("2 min ago")                     | RelativeTimestamp component (task 0063)              |

### Filters

| Filter         | Type            | Notes                               |
| -------------- | --------------- | ----------------------------------- |
| Source Account | Text input      | Filters by `filter[source_account]` |
| Contract ID    | Text input      | Filters by `filter[contract_id]`    |
| Operation Type | Dropdown/select | Filters by `filter[operation_type]` |

- Filters are additive (AND logic)
- All filters reflected in URL query params
- Filter change resets cursor (back to first page)
- No hard reloads on filter or pagination changes

### Pagination

- Cursor-based, opaque tokens
- Previous / Next only, no page numbers, no total count
- Default sort: most recent first

## Implementation Plan

### Step 1: Transactions list query hook

Create `apps/web/src/pages/transactions/useTransactionsList.ts`:

- Fetches `GET /transactions` with limit, cursor, and filter params
- Stale time: 60 seconds
- Query key: `['transactions', { cursor, filters }]`

### Step 2: Filter controls

Create `apps/web/src/pages/transactions/TransactionFilters.tsx`:

- Source account text input
- Contract ID text input
- Operation type dropdown (values from domain types)
- All values synced to URL query params via `useTableUrlState` (task 0061)
- Filter change resets cursor

### Step 3: Transactions table

Create `apps/web/src/pages/transactions/TransactionsTable.tsx`:

- Uses ExplorerTable (task 0061) with column definitions
- Columns: hash, ledger sequence, source account, operation type, status badge, fee, timestamp
- Identifier columns use linked IdentifierDisplay (task 0062)
- Status column uses StatusBadge (task 0063)
- Timestamp column uses RelativeTimestamp (task 0063)

### Step 4: Page composition

Create `apps/web/src/pages/transactions/TransactionsListPage.tsx`:

- SectionHeader: "Transactions"
- TransactionFilters above table
- TransactionsTable with data
- PaginationControls below table (task 0061)
- Loading skeleton during fetch
- Empty state when no results match filters
- Error state with retry for transient failures

## Acceptance Criteria

- [ ] Table displays columns: hash, ledger sequence, source account, operation type, status badge, fee, timestamp
- [ ] Default sort: most recent first
- [ ] Filters: source account, contract ID, operation type -- all additive
- [ ] Filters reflected in URL query params
- [ ] Filter change resets cursor to first page
- [ ] Cursor-based pagination: Previous/Next only, no total count
- [ ] No hard reloads on filter, sort, or pagination changes
- [ ] All identifiers (hash, ledger, account) linked to their detail pages
- [ ] Loading skeleton shown during fetch
- [ ] Empty state shown when no results match filters
- [ ] Error state with retry button for transient failures

## Notes

- This table serves as the reference implementation for all other list pages.
- Operation type values should come from domain types (tasks 0009-0012).
- The same table row conventions should be reused wherever transactions appear (home page, account detail, token detail, etc.).
