---
id: '0072'
title: 'Frontend: Ledgers list and detail pages'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-medium, effort-small, layer-frontend-pages]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: Ledgers list and detail pages

## Summary

Implement the Ledgers list page (`/ledgers`) and Ledger detail page (`/ledgers/:sequence`). The list page is a chain history browser optimized for monotonic sequence traversal. The detail page shows ledger metadata and paginated transactions within the ledger.

## Status: Backlog

**Current state:** Not started.

## Context

Ledgers are the fundamental time-ordered unit of the Stellar blockchain. The list page lets users browse the chain history, and the detail page shows what happened in a specific ledger. Previous/next navigation allows stepping through adjacent ledgers.

### API Endpoints Consumed

| Endpoint                 | Query Params      | Purpose                                       |
| ------------------------ | ----------------- | --------------------------------------------- |
| `GET /ledgers`           | `limit`, `cursor` | Paginated ledger list                         |
| `GET /ledgers/:sequence` | none              | Single ledger detail with linked transactions |

### Ledger List Table Columns

| Column            | Display                                                | Notes                                                              |
| ----------------- | ------------------------------------------------------ | ------------------------------------------------------------------ |
| Sequence          | Dominant visual anchor, linked to `/ledgers/:sequence` | IdentifierDisplay (task 0062). Sequence is the primary identifier. |
| Hash              | Truncated                                              | IdentifierDisplay (task 0062)                                      |
| Closed At         | Relative timestamp                                     | RelativeTimestamp (task 0063)                                      |
| Protocol Version  | Integer                                                | e.g., "21"                                                         |
| Transaction Count | Integer                                                | Number of transactions in the ledger                               |

- Default sort: most recent first
- Cursor-based pagination, no total counts

### Ledger Detail Fields

| Field             | Display                  | Notes                                |
| ----------------- | ------------------------ | ------------------------------------ |
| Sequence          | Full, prominent          | Primary identifier                   |
| Hash              | Full, copyable           | IdentifierWithCopy (task 0062)       |
| Closed At         | Full datetime + relative | RelativeTimestamp (task 0063)        |
| Protocol Version  | Integer                  | e.g., "21"                           |
| Transaction Count | Integer                  | Count of transactions in this ledger |
| Base Fee          | Value                    | Base fee for this ledger             |

### Transactions in Ledger

- Paginated table of all transactions in this ledger
- Reuses global transaction row conventions (same columns as `/transactions` list)
- Columns: hash, source account, operation type, status badge, fee, timestamp
- SectionHeader: "Transactions in Ledger #12345678"

### Previous / Next Navigation

- Previous ledger: sequence - 1 (link to `/ledgers/:prev_sequence`)
- Next ledger: sequence + 1 (link to `/ledgers/:next_sequence`)
- Stable at newest indexed ledger: "Next" disabled if no higher sequence exists
- Persistent navigation buttons at top of detail page

## Implementation Plan

### Step 1: Ledger list query hook and page

Create `apps/web/src/pages/ledgers/useLedgersList.ts` and `LedgersListPage.tsx`:

- Fetches `GET /ledgers` with limit and cursor
- Stale time: 60 seconds
- Table with columns: sequence, hash, closed_at, protocol version, tx count
- Cursor-based pagination controls
- Loading skeleton, empty state, error state

### Step 2: Ledger detail query hook

Create `apps/web/src/pages/ledger-detail/useLedgerDetail.ts`:

- Fetches `GET /ledgers/:sequence`
- Stale time: 5 minutes (immutable once closed)
- Param validation: positive integer (from task 0067)

### Step 3: Ledger detail summary

Create `apps/web/src/pages/ledger-detail/LedgerSummary.tsx`:

- Renders: sequence, hash (full, copyable), closed_at, protocol version, tx count, base fee
- Previous/next navigation buttons

### Step 4: Transactions in ledger section

Create `apps/web/src/pages/ledger-detail/LedgerTransactions.tsx`:

- Paginated transaction table (same columns as transactions list page)
- SectionHeader: "Transactions in Ledger #[sequence]"
- Uses ExplorerTable (task 0061) with transaction row conventions

### Step 5: Ledger detail page composition

Create `apps/web/src/pages/ledger-detail/LedgerDetailPage.tsx`:

- Composes: LedgerSummary, LedgerTransactions
- Each section in SectionErrorBoundary (task 0064)
- 404 state: "Ledger not found"
- Loading skeleton during fetch

## Acceptance Criteria

- [ ] Ledger list columns: sequence (dominant, linked), hash (truncated), closed_at, protocol version, tx count
- [ ] List sorted most recent first with cursor-based pagination
- [ ] Detail shows: sequence, hash (full, copyable), closed_at, protocol version, tx count, base fee
- [ ] Transactions in ledger: paginated table reusing global transaction row conventions
- [ ] Previous/next ledger navigation works correctly
- [ ] Next disabled at newest indexed ledger
- [ ] Param validation: positive integer for sequence
- [ ] 404 state: "Ledger not found"
- [ ] Loading skeleton and error states for both list and detail

## Notes

- Ledger data is immutable once the ledger is closed, so long stale times are appropriate.
- Transaction rows within a ledger should look identical to rows on the global transactions page for consistency.
- Sequence is the dominant visual anchor for each row in the list, not the hash.
