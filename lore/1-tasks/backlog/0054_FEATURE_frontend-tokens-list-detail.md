---
id: '0054'
title: 'Frontend: Tokens list and detail pages'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-medium, effort-medium, layer-frontend-pages]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: Tokens list and detail pages

## Summary

Implement the Tokens list page (`/tokens`) and Token detail page (`/tokens/:id`). Covers both classic Stellar assets and Soroban token contracts in a unified browsing surface with clear type differentiation.

## Status: Backlog

**Current state:** Not started.

## Context

The token pages must unify classic Stellar assets and Soroban token contracts into one browseable surface while making their type differences explicit. Classic assets are identified by code + issuer, Soroban tokens by contract ID. Type badges (classic / SAC / soroban) prevent user confusion.

### API Endpoints Consumed

| Endpoint                       | Query Params                                      | Purpose                                                              |
| ------------------------------ | ------------------------------------------------- | -------------------------------------------------------------------- |
| `GET /tokens`                  | `limit`, `cursor`, `filter[type]`, `filter[code]` | Paginated token list with type and code filters                      |
| `GET /tokens/:id`              | none                                              | Token detail: code, issuer/contract, type, supply, holders, metadata |
| `GET /tokens/:id/transactions` | `limit`, `cursor`                                 | Paginated transactions involving this token                          |

### Token List Table Columns

| Column               | Display                     | Notes                                                                                       |
| -------------------- | --------------------------- | ------------------------------------------------------------------------------------------- |
| Asset Code           | Text                        | Primary identifier text (e.g., "USDC", "XLM")                                               |
| Issuer / Contract ID | Truncated, linked           | Classic: issuer linked to `/accounts/:id`. Soroban: contract ID linked to `/contracts/:id`. |
| Type                 | Badge (classic/SAC/soroban) | TypeBadge (task 0043). Prevents confusion between similar names.                            |
| Total Supply         | Formatted number            | Total supply of the token                                                                   |
| Holder Count         | Integer                     | Number of accounts holding this token                                                       |

### Token List Filters

| Filter     | Type                            | Notes                     |
| ---------- | ------------------------------- | ------------------------- |
| Type       | Dropdown: classic, SAC, soroban | Filters by `filter[type]` |
| Asset Code | Text input / search             | Filters by `filter[code]` |

- Filters additive, reflected in URL
- Filter change resets cursor

### Token Detail Fields

| Field                 | Display                                    | Notes                                                    |
| --------------------- | ------------------------------------------ | -------------------------------------------------------- |
| Asset Code            | Prominent header                           | Primary token name                                       |
| Issuer (classic)      | Full, copyable, linked to `/accounts/:id`  | IdentifierWithCopy (task 0042). Only for classic tokens. |
| Contract ID (soroban) | Full, copyable, linked to `/contracts/:id` | IdentifierWithCopy (task 0042). Only for Soroban tokens. |
| Type Badge            | Prominent badge                            | TypeBadge (task 0043). Must be obvious at top of page.   |
| Total Supply          | Formatted number                           | Total token supply                                       |
| Holder Count          | Integer                                    | Number of holders                                        |
| Deployed At Ledger    | Linked to `/ledgers/:sequence`             | Only for Soroban tokens. IdentifierDisplay (task 0042).  |

### Token Metadata (when available)

| Field              | Display | Notes                                        |
| ------------------ | ------- | -------------------------------------------- |
| Name               | Text    | Full token name                              |
| Description        | Text    | Token description                            |
| Icon               | Image   | Token icon/logo. Placeholder if unavailable. |
| Domain / Home Page | Link    | External link to token's website             |

- Metadata may be partially available. Tolerate missing fields gracefully.

### Token Transactions Table Columns

Same as global transaction table conventions:
| Column | Display |
|---|---|
| Hash | Truncated, linked |
| Source Account | Truncated, linked |
| Operation Type | Label |
| Status | Badge |
| Fee | XLM |
| Timestamp | Relative |

## Implementation Plan

### Step 1: Token list query hook and page

Create `apps/web/src/pages/tokens/useTokensList.ts` and `TokensListPage.tsx`:

- Fetches `GET /tokens` with limit, cursor, type filter, code filter
- Filter controls: type dropdown, code text input
- Table with columns: asset code, issuer/contract ID, type badge, total supply, holder count
- Cursor-based pagination

### Step 2: Token detail query hooks

Create `apps/web/src/pages/token-detail/useTokenDetail.ts` and `useTokenTransactions.ts`:

- `useTokenDetail`: fetches `GET /tokens/:id`, stale time 5 minutes
- `useTokenTransactions`: fetches `GET /tokens/:id/transactions` with cursor

### Step 3: Token detail summary

Create `apps/web/src/pages/token-detail/TokenSummary.tsx`:

- Asset code as header
- Type badge (prominent, near top)
- Issuer (classic) OR contract ID (Soroban) -- full, copyable, linked
- Supply, holder count, deployed at ledger (Soroban only)

### Step 4: Token metadata section

Create `apps/web/src/pages/token-detail/TokenMetadata.tsx`:

- Name, description, icon, domain
- Graceful handling of missing fields (show what is available, hide empty sections)

### Step 5: Token transactions section

Create `apps/web/src/pages/token-detail/TokenTransactions.tsx`:

- Paginated transaction table, standard columns
- SectionHeader: "Transactions"

### Step 6: Page composition

Create `apps/web/src/pages/token-detail/TokenDetailPage.tsx`:

- Composes: TokenSummary, TokenMetadata, TokenTransactions
- Each section in SectionErrorBoundary (task 0044)
- 404 state: "Token not found"

## Acceptance Criteria

- [ ] Token list columns: asset code, issuer/contract ID, type badge (classic/SAC/soroban), total supply, holder count
- [ ] Filters: type dropdown (classic/SAC/soroban), code search. Reflected in URL.
- [ ] Token detail shows: code, issuer OR contract ID (copyable, linked), type badge (prominent), supply, holders, deployed at ledger (Soroban)
- [ ] Type badge clearly distinguishes classic, SAC, and Soroban tokens
- [ ] Metadata section tolerates partial availability (missing name/icon/description)
- [ ] Token transactions table with standard columns, cursor pagination
- [ ] Classic token issuer linked to `/accounts/:id`; Soroban contract linked to `/contracts/:id`
- [ ] 404 state: "Token not found"
- [ ] Loading skeleton and error states per section

## Notes

- Token identity is the most confusing area for users: classic tokens share codes across issuers, Soroban tokens are identified by contract. Type badges and display formatting are critical.
- The token detail page serves as a discovery path into the broader explorer via transaction links.
