---
id: '0053'
title: 'Frontend: Account detail page'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-medium, effort-small, layer-frontend-pages]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: Account detail page

## Summary

Implement the Account detail page (`/accounts/:accountId`) showing account summary, balances, and paginated transaction history. This is the canonical destination for account ID lookups from global search and linked identifiers throughout the explorer.

## Status: Backlog

**Current state:** Not started.

## Context

The account detail page provides a complete view of a Stellar account's state and activity. It serves as the landing page when users click any account identifier in the explorer or search for an account by ID.

### API Endpoints Consumed

| Endpoint                                 | Query Params      | Purpose                                              |
| ---------------------------------------- | ----------------- | ---------------------------------------------------- |
| `GET /accounts/:account_id`              | none              | Account summary: balances, sequence, first/last seen |
| `GET /accounts/:account_id/transactions` | `limit`, `cursor` | Paginated transactions involving this account        |

### Account Summary Fields

| Field             | Display                        | Notes                                                     |
| ----------------- | ------------------------------ | --------------------------------------------------------- |
| Account ID        | Full, copyable                 | IdentifierWithCopy (task 0042). Prominent at top of page. |
| Sequence Number   | Integer                        | Account sequence number                                   |
| First Seen Ledger | Linked to `/ledgers/:sequence` | IdentifierDisplay (task 0042)                             |
| Last Seen Ledger  | Linked to `/ledgers/:sequence` | IdentifierDisplay (task 0042)                             |

### Balances Section

| Field              | Display                            | Notes                                                |
| ------------------ | ---------------------------------- | ---------------------------------------------------- |
| XLM Balance        | Native balance                     | Prominent, at top of balances                        |
| Trustline Balances | Token code + balance per trustline | Each token code linked to `/tokens/:id` if available |

- Balances visually separated from transaction history
- XLM balance distinguished from trustline/token balances

### Account Transactions Table Columns

| Column          | Display                                    | Notes                             |
| --------------- | ------------------------------------------ | --------------------------------- |
| Hash            | Truncated, linked to `/transactions/:hash` | IdentifierDisplay (task 0042)     |
| Ledger Sequence | Linked to `/ledgers/:sequence`             | IdentifierDisplay (task 0042)     |
| Source Account  | Truncated, linked                          | IdentifierDisplay (task 0042)     |
| Operation Type  | Human-readable label                       | Same as global transactions table |
| Status          | Badge (success/failed)                     | StatusBadge (task 0043)           |
| Fee             | XLM amount                                 | Fee charged                       |
| Timestamp       | Relative                                   | RelativeTimestamp (task 0043)     |

- Paginated with cursor-based pagination
- Reuses global transaction row conventions

## Implementation Plan

### Step 1: Account detail query hooks

Create `apps/web/src/pages/account-detail/useAccountDetail.ts` and `useAccountTransactions.ts`:

- `useAccountDetail`: fetches `GET /accounts/:account_id`, stale time 5 minutes
- `useAccountTransactions`: fetches `GET /accounts/:account_id/transactions` with cursor, stale time 60 seconds
- Separate queries for independent section fetching

### Step 2: Account summary section

Create `apps/web/src/pages/account-detail/AccountSummary.tsx`:

- Renders: account ID (full, copyable), sequence number, first seen ledger (linked), last seen ledger (linked)
- Summary card layout at top of page

### Step 3: Balances section

Create `apps/web/src/pages/account-detail/AccountBalances.tsx`:

- XLM balance prominent at top
- Trustline/token balances listed below
- Each token code linked to token detail if available
- Visually separated from transactions section

### Step 4: Account transactions section

Create `apps/web/src/pages/account-detail/AccountTransactions.tsx`:

- Paginated transaction table with standard columns
- SectionHeader: "Transactions"
- Uses ExplorerTable (task 0041) with cursor pagination
- Reuses global transaction row conventions

### Step 5: Page composition

Create `apps/web/src/pages/account-detail/AccountDetailPage.tsx`:

- Composes: AccountSummary, AccountBalances, AccountTransactions
- Each section in SectionErrorBoundary (task 0044)
- Param validation: G... format account ID (from task 0047)
- 404 state: "Account not found"
- Loading skeleton during fetch

## Acceptance Criteria

- [ ] Account summary shows: account ID (full, copyable), sequence number, first seen ledger (linked), last seen ledger (linked)
- [ ] Balances section shows: XLM balance (prominent) + trustline/token balances
- [ ] Balances visually separated from transaction history
- [ ] Transaction table columns: hash, ledger sequence, source account, operation type, status badge, fee, timestamp
- [ ] Transactions paginated with cursor-based pagination
- [ ] Account summary and transactions fetched independently (separate queries)
- [ ] Failed transactions section does NOT collapse account summary
- [ ] Param validation: G... format for accountId
- [ ] 404 state: "Account not found"
- [ ] Loading skeleton and error states per section

## Notes

- This page is the canonical destination for all account ID links and search results.
- The account scope is intentionally limited to summary, balances, and transactions per the architecture docs.
- Transaction rows should look identical to the global transactions list page for consistency.
