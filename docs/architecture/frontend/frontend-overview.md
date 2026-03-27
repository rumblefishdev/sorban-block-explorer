# Stellar Block Explorer - Frontend Overview

> This document expands the frontend portion of
> [`technical-design-general-overview.md`](../technical-design-general-overview.md).
> It preserves the same product scope and page inventory, but specifies the frontend
> architecture in more detail so it can later serve as input for implementation task
> planning.

---

## Table of Contents

1. [Purpose and Scope](#1-purpose-and-scope)
2. [Goals](#2-goals)
3. [Context and Responsibilities](#3-context-and-responsibilities)
4. [Architecture](#4-architecture)
5. [Routing and Navigation Model](#5-routing-and-navigation-model)
6. [Routes and Pages](#6-routes-and-pages)
7. [Shared UI Elements](#7-shared-ui-elements)
8. [Data Fetching and View-State Model](#8-data-fetching-and-view-state-model)
9. [Performance and Error Handling](#9-performance-and-error-handling)
10. [Accessibility, Observability, and Delivery Notes](#10-accessibility-observability-and-delivery-notes)

---

## 1. Purpose and Scope

The frontend is the public user interface of the Stellar Block Explorer. Its role is to
present indexed Stellar and Soroban data in a form that is understandable to both casual
users and technical users, while keeping backend and indexing details hidden behind a
stable HTTP API.

This document covers the target design of the frontend application only. It does not
define backend DTOs in detail, indexing internals, or infrastructure provisioning logic,
except where those affect frontend behavior.

The current Nx workspace already reserves the frontend boundary as:

- `apps/web` - application entrypoint for the explorer web app
- `libs/ui` - reusable presentation components and frontend-only view primitives
- `libs/domain` - shared explorer concepts that may be reused by both frontend and backend
- `libs/shared` - generic non-domain utilities used across the workspace

The frontend bootstrap (React 19, Vite, MUI, React Router, TanStack Query) is in place.
This document describes the intended production architecture for that boundary. Page
components, routing, theming, and data fetching layers are implemented in dedicated tasks.

If any statement in this file conflicts with
[`technical-design-general-overview.md`](../technical-design-general-overview.md), the
general overview document takes precedence. This file is a frontend-focused refinement of
that source, not an independent redesign.

## 2. Goals

### 2.1 Primary Product Goals

- **Human-readable format** - Show exactly what occurred in each transaction. Users should
  understand payments, DEX operations, and Soroban contract calls without decoding XDR or
  raw operation codes.
- **Classic + Soroban** - Support both classic Stellar operations (payments, offers, path
  payments, etc.) and Soroban operations (invoke host function, contract events, token swaps).

### 2.2 Frontend-Specific Interpretation of Those Goals

The frontend must translate low-level explorer data into a clear visual model:

- Transaction pages must favor interpreted labels and summaries over raw protocol names.
- Advanced users must still be able to inspect exact low-level data when needed.
- Classic Stellar entities and Soroban-native entities must feel like part of one product,
  not two parallel explorers stitched together.
- Navigation must support both "known identifier lookup" flows and "browse the latest
  activity" flows.

### 2.3 Non-Goals

The frontend is not responsible for:

- parsing XDR in the browser
- deriving chain truth independently of the backend
- connecting directly to Horizon, RPC, or raw ledger ingestion services
- mutating chain state or acting as a wallet interface

## 3. Context and Responsibilities

The frontend sits at the edge of the system and communicates exclusively with the custom
NestJS REST API. It never reads from the blockchain directly.

Responsibility boundaries:

- The frontend owns routing, presentation, UI composition, search entry UX, and
  user-friendly interpretation of already-indexed data.
- The backend owns query APIs, response shaping, pagination cursors, and entity lookup.
- The indexing pipeline owns ingestion completeness, normalization, and persistence into
  PostgreSQL.

This separation is important because the frontend should remain a thin, deterministic
consumer of explorer data. Complex chain interpretation should happen upstream where it
can be tested and reused consistently.

## 4. Architecture

### 4.1 Runtime Architecture

The frontend is a React application served via CloudFront CDN. It consumes the backend
REST API with polling-based updates for new transactions and events.

The intended implementation stack is:

- **React** for route composition, page rendering, and component-based UI architecture
- **TanStack Query** for server-state fetching, caching, polling, and consistent async data
  handling across routes and shared sections
- **MUI** as the base component library, theming foundation, and accessibility-oriented UI
  primitive layer
- **React Router** for client-side routing

The frontend is a public, anonymous browser client. It must not embed API keys or other
shared secrets; API protection belongs at the API Gateway/WAF boundary, not in the bundle.

```
┌────────┐     ┌──────────────────────────────────────────────────┐
│  User  │────>│  Global Search Bar                               │
│        │     │  (contracts, transactions, tokens, accounts, …)  │
│        │     └──────────────────────────────────────────────────┘
│        │
│        │     ┌─────────────────────────────────────────────────────────────┐
│        │────>│  React Router                                               │
└────────┘     │                                                             │
               │  /                          ── GET /network/stats ──────┐   │
               │  /transactions              ── GET /transactions ───────┤   │
               │  /transactions/:hash        ── GET /transactions/:hash ─┤   │
               │  /ledgers                   ── GET /ledgers ────────────┤   │
               │  /ledgers/:seq              ── GET /ledgers/:seq ───────┤   │
               │  /accounts/:id              ── GET /accounts/:id ───────┤   │
               │  /tokens                    ── GET /tokens ─────────────┤   │
               │  /tokens/:id                ── GET /tokens/:id ─────────┤   │
               │  /contracts/:id             ── GET /contracts/:id ──────┤   │
               │  /nfts                      ── GET /nfts ───────────────┤   │
               │  /nfts/:id                  ── GET /nfts/:id ───────────┤   │
               │  /liquidity-pools           ── GET /liquidity-pools ────┤   │
               │  /liquidity-pools/:id       ── GET /liquidity-pools/:id ┤   │
               │  /search?q=                 ── GET /search ─────────────┘   │
               │                                         │                   │
               └─────────────────────────────────────────┼───────────────────┘
                                                         │
                                                         ▼
                                                 ┌──────────────┐
                                                 │   REST API   │
                                                 └──────────────┘
```

Layout direction for the frontend should be:

- data-first and explorer-oriented, with an emphasis on scanability over marketing-style
  presentation
- centered around a prominent global search input near the top of the viewport
- organized around compact summary cards near the top of the home page
- built on dense but readable latest-activity modules for ledgers and transactions
- list-heavy on collection screens, with filters and tables kept immediately discoverable
- detail-heavy on entity screens, with a concise summary first and deeper sections below

This layout direction should borrow proven explorer UX patterns without copying another
product one-to-one. The highest priority remains faithful implementation of the
information architecture, routes, and entity relationships defined in
[`technical-design-general-overview.md`](../technical-design-general-overview.md).

### 4.2 Deployment Model

- The application is delivered as static frontend assets through CloudFront.
- Client-side routing is handled in the browser by React Router.
- The API base URL is environment-specific and injected at build or deploy time.
- The browser should treat the API as the only source of explorer data.

### 4.3 Internal Frontend Layers

The frontend should be structured into four logical layers:

1. App shell
   - router, layouts, page chrome, navigation, global search entrypoint, environment banner
2. Route modules
   - page-level orchestration for each explorer screen
3. Reusable presentation primitives
   - tables, badges, copy controls, identifier renderers, timeline and graph blocks
4. View-model and API access helpers
   - typed query functions, pagination adapters, entity-specific mappers for display

The intended code placement aligns with the workspace:

- `apps/web` should contain app bootstrap, route composition, and page orchestration
- `libs/ui` should hold reusable frontend-oriented components and display models
- `libs/domain` should hold cross-app explorer concepts such as identifiers, cursor types,
  filters, and entity enums where those represent shared business concepts

### 4.4 Rendering Strategy

The baseline rendering model is a client-rendered single-page application. The explorer is
read-heavy, identifier-driven, and route-oriented, so page transitions should be fast and
preserve global UI context.

Expected rendering behavior:

- list pages render shell first, then request data
- detail pages render key layout immediately, then populate sections independently
- polling refreshes should update dynamic regions without causing full-page resets
- slow or partially unavailable sections should degrade independently where possible

### 4.5 State Strategy

The frontend should keep local state intentionally small:

- route params, filter state, sorting state, cursor state, and display mode state live in
  route-level state and URL parameters where practical
- server data is fetched on demand from the REST API
- persistent client caches should be minimal because freshness matters more than offline UX
- backend and CDN caching are preferred over long-lived browser-side caching

This keeps explorer behavior predictable and avoids stale chain views that diverge from the
indexed backend database.

## 5. Routing and Navigation Model

The routing model must support two dominant user journeys:

- direct lookup by known identifier
- exploratory browsing across latest network activity

Primary navigation entrypoints:

- header navigation for top-level entity categories
- global search bar for exact IDs and broad discovery
- deep links from any entity reference rendered inside tables or detail pages

Navigation rules:

- every major entity identifier should be linkable from anywhere it appears
- exact search hits should resolve directly to the detail page when confidence is high
- broad or ambiguous matches should remain on a grouped search results page
- list routes should preserve filters and cursor state in the URL when practical

## 6. Routes and Pages

### 6.1 Route Inventory

| Route                    | Page            | Primary API Endpoint(s)                                                     |
| ------------------------ | --------------- | --------------------------------------------------------------------------- |
| `/`                      | Home            | `GET /network/stats`, `GET /transactions?limit=10`, `GET /ledgers?limit=10` |
| `/transactions`          | Transactions    | `GET /transactions`                                                         |
| `/transactions/:hash`    | Transaction     | `GET /transactions/:hash`                                                   |
| `/ledgers`               | Ledgers         | `GET /ledgers`                                                              |
| `/ledgers/:sequence`     | Ledger          | `GET /ledgers/:sequence`                                                    |
| `/accounts/:accountId`   | Account         | `GET /accounts/:account_id`, `GET /accounts/:account_id/transactions`       |
| `/tokens`                | Tokens          | `GET /tokens`                                                               |
| `/tokens/:id`            | Token           | `GET /tokens/:id`, `GET /tokens/:id/transactions`                           |
| `/contracts/:contractId` | Contract        | `GET /contracts/:contract_id`, `GET /contracts/:contract_id/interface`      |
| `/nfts`                  | NFTs            | `GET /nfts`                                                                 |
| `/nfts/:id`              | NFT             | `GET /nfts/:id`                                                             |
| `/liquidity-pools`       | Liquidity Pools | `GET /liquidity-pools`                                                      |
| `/liquidity-pools/:id`   | Liquidity Pool  | `GET /liquidity-pools/:id`                                                  |
| `/search?q=`             | Search Results  | `GET /search`                                                               |

Each route should be implemented as a dedicated page module with:

- route parameter validation
- page-level data loading orchestration
- loading, empty, error, and success states
- a stable page header summarizing the current entity or collection

### 6.2 Home (`/`)

Entry point and chain overview. Provides at-a-glance state of the Stellar network and
quick access to exploration.

- Global search bar - accepts transaction hashes, contract IDs, token codes, account IDs,
  ledger sequences
- Latest transactions table - hash (truncated), source account, operation type, status
  badge, timestamp
- Latest ledgers table - sequence, closed_at, transaction count
- Chain overview - current ledger sequence, transactions per second, total accounts,
  total contracts

Expanded behavior:

- The home page is the fastest way to understand whether the indexer is current and the
  explorer is healthy.
- The chain overview section should behave like a dashboard summary, not a dense analytics
  page.
- The top of the page should use compact summary cards followed by latest-activity modules
  that let users immediately move into recent ledgers and transactions.
- Latest activity tables should prioritize scannability and quick click-through to detail pages.
- Polling on this page should refresh summary counts and latest rows without visually
  jumping the whole layout.

### 6.3 Transactions (`/transactions`)

Paginated, filterable table of all indexed transactions. Default sort: most recent first.

- Transaction table - hash, ledger sequence, source account, operation type, status badge
  (success/failed), fee, timestamp
- Filters - source account, contract ID, operation type
- Cursor-based pagination controls

Expanded behavior:

- The table should support rapid scanning of recent network activity.
- Filters should be additive where sensible and reflected in the URL.
- Status, operation type, and source account should be visually recognizable without
  forcing users to inspect each row deeply.
- Cursor pagination should avoid total-count assumptions so the UI remains compatible with
  large datasets.

### 6.4 Transaction (`/transactions/:hash`)

Both modes display the same base transaction details:

- Transaction hash (full, copyable), status badge (success/failed), ledger sequence
  (link), timestamp
- Fee charged (XLM + stroops), source account (link), memo (type + content)
- Signatures - signer, weight, signature hex

Two display modes toggle how **operations** are presented:

- **Normal mode** - graph/tree representation of the transaction's operation flow.
  Visually shows the relationships between source account, operations, and affected
  accounts/contracts. Each node in the tree displays a human-readable summary (e.g.
  "Sent 1,250 USDC to GD2M...K8J1", "Swapped 100 USDC for 95.2 XLM on Soroswap"). Soroban
  invocations render as a nested call tree showing the contract-to-contract hierarchy.
  Designed for general users exploring transactions.

- **Advanced mode** - targeted at developers and experienced users. Shows per-operation
  raw parameters, full argument values, operation IDs, and return values. Includes events
  emitted (type, topics, raw data), diagnostic events, and collapsible raw XDR sections
  (`envelope_xdr`, `result_xdr`, `result_meta_xdr`). All values are shown in their
  original format without simplification.

Expanded behavior:

- Normal and advanced modes should be alternate presentations over the same backend
  transaction resource, not separate data domains.
- The mode switch should be prominent and preserve the page context.
- Normal mode must prioritize clarity over completeness and should never expose raw XDR as
  the primary representation.
- Advanced mode must preserve exactness and should not silently hide null, empty, or
  protocol-level values that matter for debugging.
- Large payload areas such as XDR and event data should be collapsible to keep the screen
  usable.

### 6.5 Ledgers (`/ledgers`)

Paginated table of all ledgers. Default sort: most recent first.

- Ledger table - sequence, hash (truncated), closed_at, protocol version, transaction count
- Cursor-based pagination controls

Expanded behavior:

- This page should behave as a chain history browser, optimized for monotonic ledger
  sequence traversal.
- Sequence should be the dominant visual anchor for each row.
- Links from ledgers to transactions should be easy to discover from the detail page.

### 6.6 Ledger (`/ledgers/:sequence`)

- Ledger summary - sequence, hash, closed_at, protocol version, transaction count, base fee
- Transactions in ledger - paginated table of all transactions in this ledger
- Previous / next ledger navigation

Expanded behavior:

- Ledger detail should support quick movement through adjacent ledgers.
- Transaction rows inside a ledger should reuse the same visual conventions as the global
  transactions page wherever possible.
- Previous and next navigation should remain stable and predictable, especially around the
  newest indexed ledger.

### 6.7 Account (`/accounts/:accountId`)

Account detail view for a Stellar account.

- Account summary - account ID (full, copyable), sequence number, first seen ledger, last seen ledger
- Balances - native XLM balance and trustline/token balances
- Recent transactions - paginated table of transactions involving this account

Expanded behavior:

- This page should act as the canonical destination for exact account ID lookups from
  global search and linked identifiers.
- The summary should make account state easy to scan without requiring protocol knowledge.
- Balances should be visually separated from transaction history.
- Linked transactions should reuse the same visual conventions as the global transactions page.

### 6.8 Tokens (`/tokens`)

List of all known tokens (classic Stellar assets and Soroban token contracts).

- Token table - asset code, issuer / contract ID, type (classic / SAC / Soroban), total
  supply, holder count
- Filters - type (classic, SAC, Soroban), asset code search
- Cursor-based pagination controls

Expanded behavior:

- Classic assets and Soroban token contracts should be browseable from one surface while
  still making their type differences explicit.
- Token identity rendering must be careful because classic assets are defined by code plus
  issuer, while contracts are defined by contract ID.
- Type badges and display formatting should prevent users from confusing similarly named assets.

### 6.9 Token (`/tokens/:id`)

Single token detail view.

- Token summary - asset code, issuer or contract ID (copyable), type badge, total supply,
  holder count, deployed at ledger (if Soroban)
- Metadata - name, description, icon (if available), domain/home page
- Latest transactions - paginated table of recent transactions involving this token

Expanded behavior:

- The header must make it obvious whether the token is a classic asset, SAC, or a custom
  Soroban token contract.
- Metadata should tolerate partial availability because many assets will have incomplete
  or inconsistent descriptive fields.
- The recent transactions section should be useful as a discovery path into the broader explorer.

### 6.10 Contract (`/contracts/:contractId`)

Contract details and interface.

- Contract summary - contract ID (full, copyable), deployer account (link), deployed at
  ledger (link), WASM hash, SAC badge if applicable
- Contract interface - list of public functions with parameter names and types, allowing
  users to understand the contract's API without reading source code
- Invocations tab - recent invocations table (function name, caller account, status,
  ledger, timestamp)
- Events tab - recent events table (event type, topics, data, ledger)
- Stats - total invocations count, unique callers

Expanded behavior:

- The contract page should work as the primary developer-facing entrypoint for Soroban
  contracts.
- Interface rendering must be readable for non-authors of the contract, not just a raw ABI dump.
- Tabbed sections should separate recent usage from structural metadata.
- SAC identification must be visually clear because it materially changes user expectations
  for the contract.

### 6.11 NFTs (`/nfts`)

List of NFTs on the Stellar network (Soroban-based NFT contracts).

- NFT table - name/identifier, collection name, contract ID, owner, preview image
- Filters - collection, contract ID
- Cursor-based pagination controls

Expanded behavior:

- This view should prioritize recognition and collection browsing over raw protocol detail.
- Preview images should not block page usability if media assets fail to load.
- Contract links should remain available for users moving from NFT browsing into contract inspection.

### 6.12 NFT (`/nfts/:id`)

Single NFT overview.

- NFT summary - name, identifier/token ID, collection name, contract ID (link), owner
  account (link)
- Media preview - image, video, or other media associated with the NFT
- Metadata - full attribute list (traits, properties)
- Transfer history - table of ownership changes

Expanded behavior:

- The NFT page should clearly separate identity, media, metadata, and history.
- Media rendering should degrade gracefully for broken URLs or unsupported formats.
- Attribute lists should stay readable even when metadata is deep or irregular.

### 6.13 Liquidity Pools (`/liquidity-pools`)

Paginated table of all liquidity pools.

- Pool table - pool ID (truncated), asset pair (e.g. XLM/USDC), total shares, reserves
  per asset, fee percentage
- Filters - asset pair, minimum TVL
- Cursor-based pagination controls

Expanded behavior:

- Rows should emphasize the pool pair and current scale at a glance.
- Formatting for reserves and TVL-like values should remain consistent across the app.
- Filters should support both quick pair lookup and broader discovery of larger pools.

### 6.14 Liquidity Pool (`/liquidity-pools/:id`)

- Pool summary - pool ID (full, copyable), asset pair, fee percentage, total shares,
  reserves per asset
- Charts - TVL over time, volume over time, fee revenue
- Pool participants - table of liquidity providers and their share
- Recent transactions - deposits, withdrawals, and trades involving this pool

Expanded behavior:

- The summary area should anchor the page before users move into charts and participant data.
- Time-series charts should be secondary to factual pool metadata and must remain legible
  on smaller screens.
- Transaction history should distinguish between trade activity and liquidity management activity.

### 6.15 Search Results (`/search?q=`)

Generic search across all entity types. For exact matches (transaction hash, contract ID,
account ID), redirects directly to the detail page. Otherwise displays grouped results.

- Search input - pre-filled with current query, allows refinement
- Results grouped by type - transactions, contracts, tokens, accounts, NFTs, liquidity
  pools (with type headers and counts)
- Each result row - identifier (linked), type badge, brief context
- Empty state - "No results found" with suggestions

Expanded behavior:

- The search experience must be optimized for mixed query types, including exact hashes,
  addresses, human-readable token names, and short codes.
- Exact-match redirect behavior should only be used when ambiguity is acceptably low.
- Grouped results should help users understand what kind of entity the query most likely represents.

## 7. Shared UI Elements

Present across all pages:

- **Header** - logo, global search bar, network indicator (mainnet/testnet)
- **Navigation** - links to home, transactions, ledgers, tokens, contracts, NFTs,
  liquidity pools
- **Linked identifiers** - all hashes, account IDs, contract IDs, token IDs, pool IDs,
  and ledger sequences are clickable links to their respective detail pages
- **Copy buttons** - on all full-length identifiers
- **Relative timestamps** - "2 min ago" with full timestamp on hover
- **Polling indicator** - shows when data was last refreshed

Expanded design requirements:

- Identifier rendering should be visually consistent everywhere in the product.
- Copy interactions should confirm success without becoming noisy.
- Network environment indicators must always remain visible enough to prevent confusion
  between mainnet and testnet.
- MUI should be treated as a base component and accessibility layer, not as permission to
  ship raw default styling without explorer-specific adaptation.
- Shared UI primitives should be implemented once and reused broadly through `libs/ui`.

Recommended shared component categories:

- layout shell components
- explorer tables and section headers
- badges for status, type, and network
- identifier display and copy controls
- empty, loading, and error state components
- tabs, charts, and graph/tree visualization primitives

## 8. Data Fetching and View-State Model

The frontend should interact with backend resources at the page and section level rather
than trying to preload the entire explorer.

Guidelines:

- page routes own their top-level query lifecycle
- TanStack Query should be the default data-fetching layer for API-backed server state
  across the application
- TanStack Query also serves as the in-browser cache for server state, including request
  de-duplication, stale-state handling, and controlled background refetching
- query keys should be structured consistently by resource type, identifier, filters, and
  pagination cursor so cache behavior remains predictable across screens
- independent sections may fetch independently when this improves perceived responsiveness
- list filters, tabs, and display modes should not require hard page reloads
- cursor tokens returned by the backend should be treated as opaque
- polling should be enabled only for views where fresh activity materially improves UX

Expected page-level data patterns:

- Home: multiple lightweight summary queries, refreshed on a short interval
- List pages: one primary collection query plus filter state
- Detail pages: one primary entity query plus one or more related subresource queries
- Search: query-driven request behavior with debounce to avoid unnecessary churn

## 9. Performance and Error Handling

- **Pagination** - all list views use cursor-based pagination backed by the block
  explorer's own database
- **Loading states** - skeleton loaders for all data-dependent sections; spinner for search
- **Error states** - clear error messages for network failures, 404s (unknown
  hash/account), and rate limit responses; retry affordances where appropriate
- **Caching** - the frontend uses TanStack Query for in-browser server-state caching and
  relies on backend cache semantics at the API ingress layer to share freshness across
  clients; it should not introduce a second manual global cache layer

Expanded performance expectations:

- list tables should remain usable on large datasets without assuming full dataset counts
- route transitions should preserve shell rendering and avoid white-screen reload behavior
- expensive visualizations should load only when the relevant section is visible or needed
- polling must be conservative to avoid unnecessary API pressure

Expanded error-handling expectations:

- 404 states should explain which entity type could not be found
- transient network failures should distinguish retryable issues from invalid identifiers
- rate limit errors should explain that the failure is temporary
- partial-section failures should not collapse unrelated sections on the same page

## 10. Accessibility, Observability, and Delivery Notes

### 10.1 Accessibility

The explorer should be usable without relying on hover-only or color-only cues.

Minimum expectations:

- keyboard-accessible navigation and search
- semantic tables and headings for data-heavy pages
- textual labels for status and type badges
- sufficient contrast for timestamps, secondary metadata, and disabled states

### 10.2 Observability

Frontend telemetry should be limited to operational signals that help maintain the product:

- route-level error reporting
- failed API request tracking
- client-side rendering exceptions
- optional performance timing for major pages and heavy visual components

Observability must never become a substitute for backend correctness. It should help spot
UI failures, slow routes, and degraded search or detail views.

### 10.3 Delivery Notes

This document specifies the target product design more deeply than the current codebase.
The frontend bootstrap (React 19, Vite, MUI, React Router, TanStack Query) is complete,
but page components, routing configuration, theming, and data fetching layers are
implemented in dedicated follow-up tasks. This document should be used as the detailed
reference for that ongoing frontend implementation work.
