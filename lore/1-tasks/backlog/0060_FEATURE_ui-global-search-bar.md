---
id: '0060'
title: 'UI lib: global search bar component'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-medium, layer-frontend-shared]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# UI lib: global search bar component

## Summary

Implement the global search bar component in `libs/ui/src/search/`. This is the primary entrypoint for "known identifier lookup" journeys. It accepts diverse identifier types, provides debounced type-ahead suggestions, and on submit either redirects to an exact-match detail page or navigates to the search results page.

## Status: Backlog

**Current state:** Not started.

## Context

The global search bar is the most prominent interactive element in the explorer, visible on every page via the header. Users need to quickly look up transaction hashes, contract IDs, account addresses, token codes, ledger sequences, pool IDs, and NFT identifiers. The search bar must handle both exact-match lookups (direct redirect) and fuzzy/broad queries (search results page).

API endpoint consumed: `GET /search?q=&type=transaction,contract,token,account,nft,pool`

Accepted input types:

- Transaction hashes (64-char hex)
- Contract IDs (C... format)
- Account IDs (G... format)
- Token codes/names (string)
- Ledger sequences (positive integer)
- Pool IDs (string)
- NFT identifiers (string)

## Implementation Plan

### Step 1: Search bar UI component

Create `libs/ui/src/search/GlobalSearchBar.tsx`:

- Text input with search icon, prominent styling
- Placeholder text indicating accepted types (e.g., "Search by tx hash, account, contract, token...")
- Responsive width, fills available header space

### Step 2: Debounced type-ahead suggestions

- Debounce input at approximately 300ms before firing suggestion queries
- Display suggestion dropdown with entity type indicators
- Suggestions fetched from `GET /search?q=` with the current input
- Each suggestion shows identifier (truncated) and entity type badge

### Step 3: Submit behavior and navigation

- On form submit (Enter key): navigate immediately without waiting for suggestions
- Exact match logic: if API returns a single high-confidence result, redirect to the detail page:
  - Transaction: `/transactions/:hash`
  - Contract: `/contracts/:id`
  - Account: `/accounts/:id`
  - Token: `/tokens/:id`
  - Ledger: `/ledgers/:seq`
  - Pool: `/liquidity-pools/:id`
  - NFT: `/nfts/:id`
- Otherwise: navigate to `/search?q=<encoded_query>`
- On suggestion click: navigate directly to that entity's detail page

### Step 4: Keyboard interaction

- Arrow keys navigate suggestions
- Enter selects highlighted suggestion or submits raw query
- Escape closes suggestion dropdown
- Tab moves focus out of search bar

### Step 5: Exports

Export `GlobalSearchBar` from `libs/ui` barrel.

## Acceptance Criteria

- [ ] Search bar renders in header, visible on every page
- [ ] Accepts tx hashes, contract IDs, account IDs, token codes/names, ledger sequences, pool IDs, NFT identifiers
- [ ] Debounced type-ahead fires after approximately 300ms of inactivity
- [ ] Form submit navigates immediately (does not wait for suggestions)
- [ ] Exact match redirects to the appropriate detail page (`/transactions/:hash`, `/contracts/:id`, `/accounts/:id`, etc.)
- [ ] Non-exact matches navigate to `/search?q=`
- [ ] Keyboard accessible: arrow keys, Enter, Escape, Tab all function correctly
- [ ] Suggestion dropdown closes on blur or Escape
- [ ] Component exported from `libs/ui`

## Notes

- The search results page is task 0086; this component handles the input and navigation trigger.
- The layout shell (task 0059) provides the slot where this component is rendered.
- API interaction should use TanStack Query setup from task 0066 for suggestion fetching.
