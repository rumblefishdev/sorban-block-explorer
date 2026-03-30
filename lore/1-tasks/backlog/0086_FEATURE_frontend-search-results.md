---
id: '0086'
title: 'Frontend: Search results page'
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

# Frontend: Search results page

## Summary

Implement the Search results page (`/search?q=`) showing grouped results by entity type, exact-match redirect behavior, and inline query refinement. Optimized for mixed query types including exact hashes, addresses, token names, and short codes.

## Status: Backlog

**Current state:** Not started.

## Context

The search results page handles queries that do not resolve to a single exact match. When the search bar (task 0060) submits a query and the API indicates a confident exact match, the user is redirected directly to the detail page. Otherwise, grouped results are shown here.

### API Endpoint Consumed

| Endpoint      | Query Params                                                                            | Purpose                          |
| ------------- | --------------------------------------------------------------------------------------- | -------------------------------- |
| `GET /search` | `q` (query string), `type` (optional: transaction, contract, token, account, nft, pool) | Searches across all entity types |

### Search API Response Structure

The API returns either:

1. An exact-match redirect indicator (single high-confidence result) -- frontend navigates to detail page
2. Grouped results by entity type for display on this page

### Exact-Match Redirect Behavior

When API returns a redirect-type response:

- Transaction hash match: navigate to `/transactions/:hash`
- Contract ID match: navigate to `/contracts/:id`
- Account ID match: navigate to `/accounts/:id`
- Token match: navigate to `/tokens/:id`
- Ledger sequence match: navigate to `/ledgers/:seq`
- Pool ID match: navigate to `/liquidity-pools/:id`
- NFT match: navigate to `/nfts/:id`

Coordinate with global search bar (task 0060) which handles the initial submit.

### Grouped Results Display

Results grouped by entity type with headers and counts:

| Section         | Header Example        | Per-Result Display                                                  |
| --------------- | --------------------- | ------------------------------------------------------------------- |
| Transactions    | "Transactions (3)"    | Hash (linked to `/transactions/:hash`), type badge, brief context   |
| Contracts       | "Contracts (1)"       | Contract ID (linked to `/contracts/:id`), type badge, brief context |
| Tokens          | "Tokens (5)"          | Code + issuer/contract (linked to `/tokens/:id`), type badge        |
| Accounts        | "Accounts (2)"        | Account ID (linked to `/accounts/:id`), brief context               |
| NFTs            | "NFTs (1)"            | Name (linked to `/nfts/:id`), collection, type badge                |
| Liquidity Pools | "Liquidity Pools (1)" | Pool ID (linked to `/liquidity-pools/:id`), asset pair              |

Each result row:

- Identifier: linked to detail page
- Type badge: entity type indicator
- Brief context: enough info to distinguish results (e.g., operation type for transactions, asset code for tokens)

### Search Input on Results Page

- Pre-filled with current `q` value
- Allows inline refinement (typing updates URL, no hard reload)
- Debounced at approximately 300ms for re-search

### Empty State

- "No results found for [query]"
- Suggestions: "Try a full transaction hash, account address (G...), contract address (C...), or token name"

## Implementation Plan

### Step 1: Search query hook

Create `apps/web/src/pages/search/useSearchResults.ts`:

- Fetches `GET /search?q=` with current query
- No cache (`cacheTime: 0`)
- Debounced at approximately 300ms
- Handles redirect response: if exact match, navigate to detail page

### Step 2: Search input on results page

Create `apps/web/src/pages/search/SearchInput.tsx`:

- Pre-filled with `q` from URL
- On change: debounced URL update (`/search?q=new_value`)
- No hard reload
- Prominent position at top of results page

### Step 3: Grouped results sections

Create `apps/web/src/pages/search/SearchResultsGroups.tsx`:

- Renders sections per entity type with header and count
- Each result: identifier (linked), type badge, brief context
- Sections only rendered if they have results
- Order: Transactions, Contracts, Tokens, Accounts, NFTs, Liquidity Pools

### Step 4: Individual result row

Create `apps/web/src/pages/search/SearchResultRow.tsx`:

- Entity-type-aware rendering
- Identifier linked to appropriate detail page
- Type badge from task 0063
- Brief contextual info varies by entity type

### Step 5: Empty state

Create `apps/web/src/pages/search/SearchEmptyState.tsx`:

- "No results found for [query]"
- Suggestions for what to search: full tx hash, G.../C... addresses, token names, sequence numbers

### Step 6: Page composition

Create `apps/web/src/pages/search/SearchResultsPage.tsx`:

- Composes: SearchInput, SearchResultsGroups (or SearchEmptyState)
- Handles exact-match redirect before rendering results
- Loading spinner (not skeleton) during search
- Error state with retry for transient failures

## Acceptance Criteria

- [ ] Exact-match redirect: API redirect response navigates to correct detail page
- [ ] Grouped results: sections by entity type with headers and counts
- [ ] Each result: identifier (linked), type badge, brief context
- [ ] Search input pre-filled with `q`, allows inline refinement
- [ ] URL updates on refinement without hard reload
- [ ] Debounced re-search at approximately 300ms
- [ ] Empty state: "No results found" with search suggestions
- [ ] Loading spinner during search (not skeleton)
- [ ] Optimized for: exact hashes, G.../C... addresses, token names, short codes, sequence numbers
- [ ] Sections only shown for entity types with results

## Notes

- This page coordinates closely with the global search bar (task 0060) which handles the initial search submission.
- Exact-match redirect should only fire when ambiguity is acceptably low.
- The search API handles query classification server-side; the frontend just renders what comes back.
- No caching for search results (they should always be fresh for the current query).
