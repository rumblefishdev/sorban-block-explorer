---
id: '0047'
title: 'Frontend: router setup, route definitions, param validation'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-small, layer-frontend-pages]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: router setup, route definitions, param validation

## Summary

Set up React Router with all route definitions, parameter validation, lazy-loaded page modules, and shared error boundaries. This is the routing backbone of the explorer frontend.

## Status: Backlog

**Current state:** Not started.

## Context

The explorer has 14 routes serving collection pages, detail pages, and search. All routes live within the layout shell (task 0039). Page modules are lazy-loaded with React.lazy + Suspense to avoid loading all pages upfront. Route transitions must not cause white-screen reloads.

## Full Route Table

| Route                    | Param           | Validation                        | Page Component          |
| ------------------------ | --------------- | --------------------------------- | ----------------------- |
| `/`                      | none            | --                                | HomePage                |
| `/transactions`          | none            | --                                | TransactionsListPage    |
| `/transactions/:hash`    | hash            | 64-character hex string           | TransactionDetailPage   |
| `/ledgers`               | none            | --                                | LedgersListPage         |
| `/ledgers/:sequence`     | sequence        | positive integer                  | LedgerDetailPage        |
| `/accounts/:accountId`   | accountId       | G... format (Stellar public key)  | AccountDetailPage       |
| `/tokens`                | none            | --                                | TokensListPage          |
| `/tokens/:id`            | id              | non-empty string                  | TokenDetailPage         |
| `/contracts/:contractId` | contractId      | C... format (Stellar contract ID) | ContractDetailPage      |
| `/nfts`                  | none            | --                                | NFTsListPage            |
| `/nfts/:id`              | id              | non-empty string                  | NFTDetailPage           |
| `/liquidity-pools`       | none            | --                                | LiquidityPoolsListPage  |
| `/liquidity-pools/:id`   | id              | non-empty string                  | LiquidityPoolDetailPage |
| `/search`                | q (query param) | non-empty string                  | SearchResultsPage       |

## Implementation Plan

### Step 1: Router configuration

Create `apps/web/src/router.tsx`:

- React Router `createBrowserRouter` with all routes defined
- Layout route wrapping all pages in AppShell (task 0039) with `<Outlet>`
- 404 catch-all route for unmatched paths

### Step 2: Lazy loading

- Each page component loaded via `React.lazy(() => import('./pages/...'))`
- Wrapped in `<Suspense>` with loading fallback (skeleton from task 0044)
- Loading fallback renders within the shell -- no white-screen

### Step 3: Route parameter validation

Create `apps/web/src/router/paramValidation.ts`:

- `validateTransactionHash(hash)`: 64-character hex string (`/^[a-fA-F0-9]{64}$/`)
- `validateLedgerSequence(seq)`: positive integer (`/^\d+$/`, > 0)
- `validateAccountId(id)`: G... format (starts with 'G', valid Stellar public key length)
- `validateContractId(id)`: C... format (starts with 'C', valid Stellar contract ID length)
- `validateNonEmpty(value)`: non-empty string
- Invalid params render the 404/not-found state (from task 0044)

### Step 4: Route-level error boundary

Create `apps/web/src/router/RouteErrorBoundary.tsx`:

- Catches uncaught errors at the route level
- Renders error state within the shell (header/nav remain visible)
- Reports route path, error, and component stack for observability (task 0059)
- Provides "Go Home" or "Retry" navigation options

### Step 5: Router provider integration

Wire router into `apps/web/src/main.tsx`:

- `<RouterProvider router={router} />`
- Wrapped inside TanStack Query provider (task 0046)
- Wrapped inside MUI ThemeProvider (task 0077)

## Acceptance Criteria

- [ ] All 14 routes defined and mapping to correct page components
- [ ] All page components lazy-loaded with React.lazy + Suspense
- [ ] Suspense fallback renders within the layout shell (no white-screen)
- [ ] Route params validated: hash (64-char hex), sequence (positive int), accountId (G...), contractId (C...), search q (non-empty)
- [ ] Invalid params render entity-type-specific not-found state
- [ ] 404 catch-all for unmatched routes
- [ ] Route-level error boundary catches uncaught errors and renders within shell
- [ ] Router wrapped in QueryProvider and ThemeProvider

## Notes

- Page components are implemented in tasks 0048-0058. This task sets up the routing skeleton with placeholder/empty page modules.
- Param validation functions should be reusable by the identifier display components (task 0042) and `libs/domain`.
- The router must be compatible with static SPA hosting on CloudFront (all routes serve index.html, client-side routing handles path resolution).
