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

- `web` - application entrypoint for the explorer web app
- `libs/ui` - reusable presentation components and frontend-only view primitives
- `libs/api-types` - generated TypeScript types, fetch SDK, and TanStack Query hooks
  derived from the Rust API's OpenAPI spec (single source of truth for API contracts)

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
REST API (axum). It never reads from the blockchain directly.

Responsibility boundaries:

- The frontend owns routing, presentation, UI composition, search entry UX, and
  user-friendly interpretation of already-indexed data.
- The backend owns query APIs, response shaping, pagination cursors, and entity lookup.
- The indexing pipeline owns ingestion completeness, normalization, and persistence into
  ClickHouse.

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
- **`@rumblefish/api-types`** generated from the backend OpenAPI 3.1 spec
  (`@hey-api/openapi-ts`); provides the TypeScript types, fetch SDK, and TanStack Query
  hooks consumed by the rest of the frontend. The frontend never hand-writes API
  types — see [Section 4.5](#45-api-types-and-codegen) below.

The frontend is a public, anonymous browser client. It must not embed API keys or other
shared secrets; API protection belongs at the ingress boundary — API Gateway throttling
plus the Cloudflare edge in front of the API hostname — not in the bundle. Note that the
Cloudflare Turnstile widget the SPA loads gates **API access**: the widget produces a
challenge token, the SPA posts it to `/auth/session`, and the backend verifies it against
Cloudflare's siteverify endpoint before minting a free-tier session JWT
(`crates/api/src/auth/`). The token and the JWT are different things — the widget never
issues the session. This is also not Cloudflare fronting the frontend's own domain, which
sits on Route 53 with no edge filter.

```
┌────────┐     ┌──────────────────────────────────────────────────┐
│  User  │────>│  Global Search Bar                               │
│        │     │  (contracts, transactions, assets, accounts, …)  │
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
               │  /assets                    ── GET /assets ─────────────┤   │
               │  /assets/:id                ── GET /assets/:id ─────────┤   │
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

- `web` should contain app bootstrap, route composition, and page orchestration
- `libs/ui` should hold reusable frontend-oriented components and display models

### 4.4 Rendering Strategy

The baseline rendering model is a client-rendered single-page application. The explorer is
read-heavy, identifier-driven, and route-oriented, so page transitions should be fast and
preserve global UI context.

Expected rendering behavior:

- list pages render shell first, then request data
- detail pages render key layout immediately, then populate sections independently
- polling refreshes should update dynamic regions without causing full-page resets
- slow or partially unavailable sections should degrade independently where possible

### 4.5 API Types and Codegen

The frontend imports all API request/response types, the fetch SDK, and TanStack Query
hooks from `@rumblefish/api-types`. That package is generated end-to-end from the Rust
API:

1. The `api` crate (Rust) annotates handlers and DTOs with `utoipa` derive macros and
   exposes them as an OpenAPI 3.1 spec.
2. A secondary binary `cargo run -p api --bin extract_openapi` prints the spec to
   stdout; CI / developers redirect it to `libs/api-types/src/openapi.json`.
3. `@hey-api/openapi-ts` reads that spec and emits types, a typed fetch client, and
   TanStack Query hooks under `libs/api-types/src/generated/` — committed to the repo
   so frontend developers do not need a Rust toolchain.
4. The Nx target `@rumblefish/api-types:check-generated` reruns the pipeline in CI and
   fails the build if the committed spec or generated files have drifted from the
   current Rust source. This is the single guard that keeps frontend types in sync
   with the API.

Authoring rule: changes to API DTOs, request params, or routes must be made in the
Rust crate; the frontend regenerates and consumes the result. Hand-edited types in
`libs/api-types/src/generated/` will be overwritten on the next regeneration.

### 4.6 State Strategy

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
| `/assets`                | Assets          | `GET /assets`                                                               |
| `/assets/:id`            | Asset           | `GET /assets/:id`, `GET /assets/:id/transactions`                           |
| `/contracts/:contractId` | Contract        | `GET /contracts/:contract_id` (+ `/interface`, `/invocations`, `/events`)   |
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

- Hero - headline, tagline, and a large global search bar accepting transaction hashes,
  contract IDs, token codes, account IDs, ledger sequences (submits to `/search`). The
  global search bar is also always present in the header.
- Chain overview - current ledger sequence (with a live indicator), transactions per
  second, total accounts, total contracts
- Latest transactions table - hash (truncated), source account, operation type, status
  badge, relative + absolute timestamp; section header carries a polling indicator and a
  "View All" link
- Latest ledgers table - sequence, hash (truncated), closed_at, protocol version,
  transaction count

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

**Net settled — REMOVED (2026-09-04).** The list rows no longer carry a `values`
field. The per-(transaction, asset) aggregate had no direction and no account, so
on an account page an inbound and an outbound transfer rendered identically; it is
replaced by a lossless per-transfer design.

Expanded behavior:

- The table should support rapid scanning of recent network activity.
- Filters should be additive where sensible and reflected in the URL.
- Status, operation type, and source account should be visually recognizable without
  forcing users to inspect each row deeply.
- Cursor pagination should avoid total-count assumptions so the UI remains compatible with
  large datasets.

### 6.4 Transaction (`/transactions/:hash`)

One progressive view (ADR 0032 update, 2026-07: the former normal/advanced
toggle is removed — everything it gated is reachable through per-section
disclosures on a single page).

Transaction-level sections:

- Summary: hash (full, copyable), status chip, a one-phrase story chip
  classifying the whole transaction ("Swap · 4 ops", "Contract call") when the
  operation shapes allow it, ledger (link), timestamp, fee, memo, source
  account; for fee-bump envelopes also the fee source account and the inner
  transaction hash (copy-only — inner hashes are not indexed as pages).
- A failure banner when `successful` is false: atomicity wording ("no
  operation was applied") plus the fail reason — the first failing
  operation's per-op result code (`heavy.operations[].result_code`, e.g.
  `Create Account #2 — LOW_RESERVE`, with a count when more ops failed);
  when no per-op array exists (validation-level failures, older cached
  responses) the raw tx-level `heavy.result_code` shows instead.
- Signatures; Events (all decoded events, collapsed by default); Raw data
  (`envelope_xdr`, `result_xdr`, `result_meta_xdr` as collapsible rows, each
  with a Stellar Lab decode deep link).

Operations render as a master-detail: a picker (per-type icon + label per
operation, selection deep-linked as `#op-N`) and **one operation card** for the
selected operation. The picker appears only above one operation — 84.6 % of
mainnet transactions carry exactly one (5 369 984 of 6 349 043, measured on
production over ledgers 63 680 000–63 700 000, deduplicated), and an index of a
single row is pure width tax.

`#op-N` is 1-based and user-supplied, so `useSelectedOp` resolves it against
the decoded operation list and owns the result, the way `useTableUrlState` owns
`sort`/`dir`: the index handed to the section always addresses an existing
operation, and no consumer carries a range guard. A fragment naming an
operation the transaction does not have resolves to the first one **with no
notice**: the card numbers itself from `application_order`, not from the
fragment, so the reader gets a correctly numbered operation and nothing is
hidden or mislabelled. Reaching that state needs a hand-edited URL — `#op-N` is
written in one place, from a picker click, and an operation count never changes,
so a shared link that worked once keeps working. The fragment itself is left
alone, so the address bar still shows what was asked for. While the archive
fetch is in flight or has failed the list length is unknown, and an unresolvable
fragment then makes no claim at all rather than asserting a count nobody
measured (0377, 0482).

The card shows:

- a truthful per-type headline sentence built from `heavy.details` (light
  fields as degraded fallback; unknown types fall back to the type label);
- a route strip for path payments — asset chips with per-hop amounts chained
  from `claimedAtoms`, flagged as partial when the route also crossed the
  order book (those fills are not in the LP-only atoms);
- an **execution trace** for contract invocations, rebuilt client-side from
  `heavy.diagnostic_events` (`fn_call`/`fn_return` stack walk): the calls
  that actually ran, nested, with per-node args/return behind a disclosure,
  the contract events attached to the call that raised them, and — on a
  failed transaction — a truthful "stopped here" on the calls that never
  returned. `core_metrics` host counters are excluded;
- a **Resources** disclosure carrying those `core_metrics` counters — all
  nineteen, full integers with US grouping, on any Soroban operation
  (footprint extend/restore are metered too and raise no `fn_call`, so the
  counters are the only thing the host says about them);
- when a transaction carries no diagnostic events, the card falls back to
  the **authorized-calls** tree fed by `heavy.operation_tree`. That is the
  auth-entry tree (what the transaction was signed to do), and the backend
  stamps every node with the whole transaction's verdict — so the UI
  deliberately renders no per-node ✓/✗ there;
- the operation's own events, matched via `XdrEventDto.op_index`
  (`application_order - 1`);
- an "Operation details" disclosure with every raw `details` key — exactness
  preserved; nothing null/empty that matters for debugging is hidden;
- on a failed transaction the card dims and carries a "not applied" label.

Consumed heavy fields: `operations[].details`, `operations[].result_code`
(per-op result names straight from the XDR library — the fail-reason source),
`operation_tree`, `diagnostic_events` (execution trace + Resources counters),
`contract_events[].op_index`, `result_code`, `fee_bump_source`, `signatures`,
`envelope_xdr`, `result_xdr`, `result_meta_xdr`.
Large payload areas stay collapsible.

#### Events section — one list, the consensus stream (issue #378)

The transaction's **Events** card lists `heavy.contract_events` only: the
tx-level and per-operation containers, hashed into the ledger, which is what
CAP-67 and `getEvents` mean by the events of a transaction. Its `Where` column
names the raising operation (`op_index`) or, for tx-level events, the CAP-67
`stage` (`before all txs` / `after all txs`) — `event_index` follows XDR
container order, so the fee refund is numbered ahead of the operation it
refunds and the number alone would read as a timeline it is not.

`heavy.diagnostic_events` sits under its own disclosure, **raw** — the call
trace, contract logs, failure diagnostics, and the byte-identical copies of the
contract's own events that the container carries when diagnostic mode is on
(always, for the archive we read). The copies are not trimmed for tidiness: the
execution trace on the operation card is a readable rendering of these rows,
not a replacement for them, and the raw table stays as the record it derives
from.

`core_metrics` is the single omission, and only because `readResourceCounters`
is **total** — it cannot silently drop a counter, whatever shape the value
arrives in — so the Resources disclosure is a complete record of them and
nothing leaves the page. On a minimal Soroban transaction they are 19 of 24
rows, and listing them here would bury the four that describe the execution.

The two never merge, and only the consensus stream is counted. That merge was
the whole of issue #378: one list and one number for two different records, so
a transaction that emitted 3 events advertised 27 and printed its transfer
twice. Separating the records fixes it; hiding rows was never needed.

### 6.5 Ledgers (`/ledgers`)

Paginated table of all ledgers. Default sort: most recent first.

- Ledger table - sequence, hash (truncated), closed_at, protocol version, transaction count with its successful/failed split (task 0445; the split is nullable — the plain total renders when the API has no split to report)
- Cursor-based pagination controls

Expanded behavior:

- This page should behave as a chain history browser, optimized for monotonic ledger
  sequence traversal.
- Sequence should be the dominant visual anchor for each row.
- Links from ledgers to transactions should be easy to discover from the detail page.

### 6.6 Ledger (`/ledgers/:sequence`)

- Ledger summary - sequence, hash, closed_at, protocol version, transaction count with its successful/failed split (task 0445), base fee
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
- **Assets** - native XLM plus every trustline, INCLUDING the ones standing at zero
- **Signers** - the signer set and the three thresholds
- Recent transactions - paginated table of transactions involving this account

Expanded behavior:

- This page should act as the canonical destination for exact account ID lookups from
  global search and linked identifiers.
- The summary should make account state easy to scan without requiring protocol knowledge.
- Assets should be visually separated from transaction history.
- Linked transactions should reuse the same visual conventions as the global transactions page.

**Assets, not "Balances" (issue #377, task 0463).** A trustline standing at zero is a
real thing the account holds — permission granted to an issuer — and the read path
selects on `closed_at_ledger = 0`, never on the amount, so those rows are shown. The
card is therefore named for what it lists, and the meta states two numbers apart:
`5 assets · 2 with a balance` (the second clause is dropped when every asset is funded,
where it only restates the first).

Order comes from the API and is NOT re-sorted client-side, because pagination has to cut
where the server cut: native pinned first, then funded before empty, then size, then
recency. The middle key is the load-bearing one — recency alone strands a real holding
under thousands of empty rows.

Paged in the browser rather than by cursor: the whole set arrives with the account, which
is what lets the caption state an exact position (`1–20 of 1015`) where the cursor
sections must say "Latest results". Position lives in `?assets=`, distinct from the
`?cursor=` the transactions section owns on the same page, so Back after opening an asset
returns to the page you were on. It is clamped, never trusted. The pager appears only
above one page — 99% of accounts hold 18 assets or fewer and should see no machinery.
Each row carries the type chip AND, separately, a `SAC` tag when the asset has a deployed
Stellar Asset Contract — two orthogonal axes, the same pairing `/assets` renders.

**Signers (issue #377, task 0463).** The account's own key is composed in as the FIRST
row with a badge: the ledger keeps it out of `signers` and expresses its weight as
`master_weight`, so a page rendering only the list reads 3-of-4 where the chain says
3-of-5. Horizon synthesises that entry server-side; we assemble it in the view and leave
the API saying what the chain says. `master_weight = 0` renders as `master key — disabled`
— a permanently disabled key, not a low-weight signer, and the second most common shape
on pubnet. An account no key can sign for gets its own marker. The footer states total
weight beside all three thresholds rather than one "N of M", which would pick one
threshold and hide two. `preauth_tx` and `hash_x` signers render unlinked; they are not
accounts.

A missing signing configuration is stated as a fact about the ledger, not as thin
coverage: after the checkpoint seed every live account has one, and accounts without a
row probe ABSENT on chain. Which of the three no-configuration states the card shows is
decided in this order: a **classic** holding with no configuration is the warning (see
below); otherwise `deleted` says the account was closed; otherwise there is no account at
this address, and when the page is also showing a Soroban balance the card says why that
is not a contradiction — a SEP-41 balance lives in the token contract's storage and needs
no account (1,325 such addresses on pubnet carry sequence number 0).

The warning reads **classic** holdings only, never all of them. A classic trustline cannot
exist without an `AccountEntry`, so one with no signing configuration means the gap is
ours; a Soroban balance implies nothing, because it outlives `account_merge` and can
predate any account. Reading all holdings fired the alarm on 10,713 accounts whose data was
correct (100% Soroban rows, 350/350 probed ABSENT on chain while 60/60 of their token
balances probed PRESENT with matching amounts). Restricted to classic it measures zero and
still cannot miss a live account, since every live account carries a native XLM row.

### 6.8 Assets (`/assets`)

List of all known assets (native XLM, classic credit assets, SACs, and Soroban-native assets).

- Asset table - asset code, issuer / contract ID, type badge (+ a separate "SAC"
  tag when the asset carries a deployed SAC), total supply, holder count
- **What names an asset** (`assetDisplayCode`, task 0472). Only classic credit
  assets carry an `asset_code`; native XLM and Soroban tokens return `null` and
  need the rest of the row to name them. One shared rule serves the list cell,
  the detail title/breadcrumb and the letter avatar: `asset_type_name === 'native'`
  → `XLM`, else `asset_code`, else the on-chain SEP-41 `symbol` (task 0304), else
  nothing (a dash in the table, "Asset" as a page title). It mirrors the pool-leg
  rule (`assetLegLabel`); the avatar takes the SAME label, so its letter
  matches the name beside it; when nothing names the asset the avatar keeps
  the honest `?`
- Filters - type chips (All types, Classic credit, Soroban) + a separate "Has SAC"
  property toggle, asset code search
- Cursor-based pagination controls

Expanded behavior:

- All asset types should be browseable from one surface while still making their type
  differences explicit.
- Asset identity rendering must be careful because classic assets are defined by code plus
  issuer, while contracts are defined by contract ID.
- Type badges and display formatting should prevent users from confusing similarly named assets.
- **SAC is a facet, not a type** ([ADR 0051](../../../lore/2-adrs/0051_sac-as-facet-of-classic-credit.md),
  task 0339). The API no longer returns `asset_type_name = "sac"`. Type and SAC
  are orthogonal axes: the frontend ALWAYS shows the type badge (Native / Classic credit
  / Soroban, `assetTypeMeta`) and ADDS a separate "SAC" property tag (`SAC_TAG`)
  only when the SAC is deployed (`sac_deployed`). Likewise "SAC" is not a type
  chip but a separate **"Has SAC" property toggle** — it sends `filter[sac]=true`
  (deployed SACs only), not `filter[type]=sac`. The SAC contract link (list +
  detail) renders from the re-derived `sac_contract_id` and is
  **deployment-aware**: linked to the contract page only when `sac_deployed`,
  otherwise shown as a reserved (non-clickable) address with a "not deployed"
  note (subsumes the old link-guard task 0337).

### 6.9 Asset (`/assets/:id`)

Single asset detail view.

> **Routing (`:id` token, task 0243 / ADR 0051).** The numeric surrogate was dropped
> (PR #175); `:id` is a single canonical token — a contract StrKey (`C…`, Soroban), a
> `CODE-ISSUER` composite (classic credit incl. **SAC-wraps** — a SAC is a facet, not a
> separate identity, ADR 0051), or the reserved `native` literal (the classic XLM
> singleton). The API returns this same token in `AssetItem.id`, so the frontend never
> builds the URL from parts — it echoes `AssetItem.id` (e.g. `routeForHit`, the list-table
> row link, and the LP-leg link all do exactly this). A SAC's `C…` still resolves as a
> deep-link (`fetch_by_contract_id` matches the `sac_contract_id` facet), for back-compat
> with links minted before the change. Search asset hits carry the token in `route_token`
> (the displayed `identifier` stays the asset code).

- Asset summary - asset code, issuer or contract ID (copyable), type badge, total supply,
  holder count, deployed at ledger (if Soroban)
- Metadata - name, description, icon (if available), domain/home page
- Latest transactions - paginated table of recent transactions involving this asset

Expanded behavior:

- The header must make it obvious whether the asset is native XLM, a classic credit asset,
  a SAC, or a Soroban-native asset.
- Metadata should tolerate partial availability because many assets will have incomplete
  or inconsistent descriptive fields.
- The recent transactions section should be useful as a discovery path into the broader explorer.

Data sourcing for the Metadata section: `name` and `icon` come from the `assets`
DB row (`name` is shown on list too; `icon` is the list-thumbnail column).
`description` and `domain/home page` are fetched from per-entity S3 at
`s3://<bucket>/assets/{id}.json` — detail-only content that is off-chain SEP-1
enrichment, not derived from XDR. See ADR 0037 / task 0164 for the rationale and
the S3 layout. Missing S3 object simply renders those two fields as blank,
consistent with the "tolerate partial availability" expectation above.

### 6.10 Contract (`/contracts/:contractId`)

Contract details and interface.

- **Contract header names what the contract IS** (`contractFace`, task 0472), for
  every class, not just SAC: `Stellar Asset Contract · CODE` → the mirrored asset,
  `Fungible` → its own asset page (`/assets/{C…}` resolves the contract StrKey),
  `NFT` → its collection (`/nfts?contract={C…}`), `Other` → unlinked. Only SAC
  carries the code suffix — `sac_asset` is the one identity the contract endpoint
  returns; a Fungible's symbol and a collection's name live on other endpoints
- Contract summary - contract ID (full, copyable), deployer account (link), deployed at
  ledger (link), WASM hash, SAC badge if applicable. A SAC additionally shows the
  asset it mirrors (tasks 0441 + 0472) as two SEPARATE labelled cells in one row —
  `Asset` (code, linked to its asset page) and `Issuer` (account, linked) — the same
  two-cell shape as the `Deployer` / `Deployed at ledger` row above it. Native XLM
  renders the `Asset` cell only (`XLM` → `/assets/native`); it has no issuer. On the
  contracts LIST the same data renders a linked `SAC · CODE` chip that REPLACES the
  `Token` type chip on SAC rows (task 0472 — on prod Token ⟺ SAC exactly, so the
  pair was redundant), with the issuer in a tooltip/aria-label; the type-filter
  chip for that bucket is labelled `SAC` (UI label only — the API still takes
  `filter[type]=token`). Both come from the `sac_asset` API field. A non-SAC
  contract renders its plain type chip and no asset row; a SAC whose `sac_asset`
  is `null` (unresolvable facet) keeps the bare, unlinked `SAC` badge
- Contract interface - list of public functions with parameter names and types, allowing
  users to understand the contract's API without reading source code. SAC and pre-upload
  contracts carry no WASM interface metadata and show an empty state
- Invocations tab - recent invocations table (transaction hash, caller account, status,
  ledger, timestamp). The appearance index carries no per-call function name — call
  detail is XDR-only (ADR 0034), so the transaction hash links to the full detail
- Events tab - recent events table (event type, topics, data, ledger). Only `contract`
  and `system` events are returned; the diagnostic-events container is dropped
  server-side (task 0182)
- Stats - recent invocations and unique callers over a rolling window (`stats_window`,
  e.g. last 7 days) — the API exposes windowed counts, not full-history totals

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
  per asset, fee percentage, participant count (active LP positions; task 0246)
- Filters - asset (`filter[asset_code]`, case-insensitive **substring** of either
  leg, so `USD` matches the `USDC` pools; `A/B` is a pair query requiring both
  codes in either order; native legs match on `XLM` despite storing an empty
  code; task 0440). The same box also accepts a pool **identifier** in the `L…`
  SEP-23 form and then selects that one pool — pasting an id used to be matched
  as an asset-code substring, so the page answered "no pools" about a pool that
  exists (task 0470). Per-leg `(code, issuer)` exact match is the alternative
  mode. `filter[min_tvl]` is **rejected with a 400**: pool TVL is computed at
  read time, so there is nothing to pre-filter on
- Cursor-based pagination controls

Expanded behavior:

- Rows should emphasize the pool pair and current scale at a glance.
- Formatting for reserves and TVL-like values should remain consistent across the app.
- Filters should support both quick pair lookup and broader discovery of larger pools.
- **Every leg links to its asset** (`legHref`, list + detail): native →
  `/assets/native` (the canonical token since task 0243 — the older "native has
  no on-chain address" carve-out left XLM as the app's only dead leg, fixed in
  task 0472), SAC mirror → `/assets/{C…}`, classic → `/assets/{CODE-ISSUER}`.
  Only genuine schema drift (no type, no code/issuer) renders unlinked text.

### 6.14 Liquidity Pool (`/liquidity-pools/:id`)

- Pool summary - pool ID (full, copyable), asset pair, fee percentage, total shares,
  reserves per asset, participant count (task 0246)
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
- Results grouped by type - transactions, contracts, assets, accounts, NFTs, liquidity
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
- **Navigation** - links to home, transactions, ledgers, assets, contracts, NFTs,
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

### 7.1 `libs/ui` Implementation Layout

The shared UI library is structured by concern, not by alphabetical component
name. Each subtree exports through a barrel; the top-level `libs/ui/src/index.ts`
re-exports the public surface.

| Path                                   | Role                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `theme/colors.ts`                      | Raw color scales (base / gray / accent / status / chip pair) + semantic tokens (text, surface, stroke) split into `colorsLight` and `colorsDark`. Tokens sourced verbatim from the Figma design-system JSON.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `theme/typography.ts`                  | 49 typography variants — 24 headings (Mona Sans, h1–h6 × 4 weights), 15 body (Inter, 5 sizes × 3 weights), 10 mono (JetBrains Mono, 5 sizes × 2 weights). Heading sizes responsive at h3–h6.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `theme/overrides.ts`                   | MUI `Components<Theme>` overrides for Button, Chip, Switch, Checkbox, Radio, Slider, TextField, Outlined/Filled inputs, Select, Menu, Paper, Card, Tabs, Table family, Pagination, Tooltip. Driven entirely by semantic palette tokens so light/dark theming is mode-aware.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `theme/theme.ts`                       | `createExplorerTheme(mode)` assembles palette + typography + shape + shadows + overrides via `createTheme`. Module augmentation lives in `theme/types.ts` so custom palette / typography / chip color / size keys are typed everywhere.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `theme/ThemeProvider.tsx`              | `ExplorerThemeProvider` wraps MUI `ThemeProvider` + `CssBaseline`. Exports `useColorMode()` hook that persists `light`/`dark` to `localStorage` and defaults to `prefers-color-scheme`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `components/Chip.tsx`                  | Thin wrapper around MUI `Chip` adding a `dot` shortcut prop. The 8px dot indicator is a private helper inlined in the same file — not part of the public API.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `states/skeletons/`                    | `TableSkeleton` / `CardSkeleton` / `DetailSkeleton` / `SearchSpinner`. Skeletons are parameterised (rows/cols/lines/sections) so page tasks tune them to match their real layouts; SearchSpinner is `CircularProgress` in a centred min-height container to prevent layout shift.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `states/errors/`                       | `NotFoundState` (entity-typed: transaction / account / contract / ledger / operation / asset / liquidity-pool / generic), `TransientErrorState` (retryable boolean, swaps title + hides retry button when non-retryable), `RateLimitState` (optional auto-retry countdown), `GenericErrorState`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `states/empty/`                        | Generic `EmptyState` with icon / title / description / action / meta props and three variants (default / warning / error). Entity-specific copy is passed at call sites — no per-entity wrapper components.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `states/SectionErrorBoundary`          | Class component that catches render errors inside a single section and renders `GenericErrorState` inline. Siblings on the same page stay rendered. Accepts `sectionName` for observability and an optional custom `fallback` render function.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `states/classifyError.ts`              | Pure utility mapping a `Response`-shaped error or `Error` to `'not-found' \| 'rate-limit' \| 'transient' \| 'validation' \| 'unknown'`. Used by TanStack Query error handlers to pick the right state component.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Network / status / type badges         | All rendered via `Chip` at call sites — no per-entity wrapper components. Network: `<Chip color="blue" label="Mainnet">` / `<Chip color="warning" label="Testnet">`. Status: `<Chip color="success" dot label="Success">` / `<Chip color="error" dot label="Failed">`. Type (`assetType.ts`): `<Chip color="blue" label="Native">` / `<Chip color="neutral" label="Classic credit">` / `<Chip color="emerald" label="Soroban">`, plus a separate SAC property tag `<Chip color="brown" label="SAC">` shown in addition to the type badge when `sac_deployed` (ADR 0051). **Chip glossary (task 0472): one colour per meaning app-wide** — brown is reserved for the SAC facet (assets tag, contracts chips incl. the detail header, balances), emerald means a Soroban token wherever it appears (type "Soroban" on /assets and balances, class "Fungible" on /contracts — two axis-correct labels, one colour), violet is NFT, neutral is classic credit, subtle is the unclassified `Other` bucket. The Chip color overrides in `theme/overrides.ts` already match Figma's badge palettes 1:1. |
| `timestamps/RelativeTimestamp`         | Renders a timestamp as relative time ("2 min ago") with the full ISO shown on hover via tooltip. Inside a `LiveNowProvider` (home live tables) `now` is refetch-synced — labels age atomically with each poll and show exact seconds; outside, the shared 1s `useNow` tick drives it with sub-minute 5s buckets so non-polled tables don't flicker a new value in every row every second.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `timestamps/PollingIndicator`          | "Checked Xs ago" + refresh icon for polling-enabled sections. `lastUpdated` is TanStack Query's `dataUpdatedAt` — when the last successful poll response arrived (a polling heartbeat), deliberately NOT data freshness: per-row age is the Time column's job and chain staleness is the LiveIndicator's. An optional `retrying` flag appends "· retrying…" while a failed poll leaves stale rows on screen.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `timestamps/useNow` + `formatRelative` | Shared helpers — `useNow(intervalMs)` returns a `Date` that re-renders every interval; `formatRelative(then, now, precision)` is a pure function returning the short relative string; `'bucket'` (tick-driven callers) renders "just now" under 10s then 5s buckets ("10s ago" … "55s ago"), `'exact'` (refetch-synced callers inside `LiveNowProvider`) renders exact seconds; both converge to "2 min ago" / "1 hour ago" / "3 days ago". `LiveNowProvider` (`timestamps/LiveNow.tsx`) supplies a refetch-synced `now` from the query's `dataUpdatedAt` with a 10s fallback tick for stalled feeds.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |

Fonts (Mona Sans, Inter, JetBrains Mono — variable `.ttf`) are self-hosted at
`web/public/fonts/` and registered via `web/src/styles/fonts.css` with
`format('truetype-variations')` so the weight axis works.

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

### 8.1 Implementation Layout

The data layer lives in `web/src/api/` and is built on top of the `@hey-api/openapi-ts`
`@tanstack/react-query` plugin output in `libs/api-types`. Page tasks consume hook
wrappers from this folder — they do not call `fetch` or the generated SDK directly.

| File                            | Role                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `config.ts`                     | Reads + validates `VITE_API_BASE_URL` at startup (no API keys are ever read; the frontend is anonymous).                                                                                                                                                                                                                                                                                     |
| `client.ts`                     | Configures the generated `@hey-api/client-fetch` instance with the base URL and registers an error interceptor that stamps `status` and wraps non-`Error` throws so downstream code always sees a real `Error` with `.status` / `.body`.                                                                                                                                                     |
| `QueryProvider.tsx`             | Constructs the singleton `QueryClient` (60s default `staleTime`, 5min `gcTime`, `refetchOnWindowFocus`, retry once on 5xx/network, never on 4xx) and mounts `ReactQueryDevtools` in dev.                                                                                                                                                                                                     |
| `polling.ts`                    | Per-resource cache policies: `livePolicy` + `midpointPollDelay` (adaptive per-ledger cadence for the home feeds _and_ `useNetworkStats` — each fetch aims at the midpoint of the next inter-block gap, anchored on the newest row's close time; server cooperates with `Cache-Control: max-age=0` on these endpoints), `listPolicy` (60s), `detailPolicy` (5min), `searchPolicy` (no cache). |
| `queryKeys.ts`                  | `invalidateResource(qc, resource)` / `matchResource(resource)` predicates over the codegen `_id`-prefixed keys (resource → list of SDK ids).                                                                                                                                                                                                                                                 |
| `hooks/use*.ts`                 | Thin wrappers around generated `*Options` / `*InfiniteOptions` that spread the matching policy and shape the public hook signature.                                                                                                                                                                                                                                                          |
| `router/routes.ts`              | Consumer-facing URL helpers (`routes.home`, `routes.transaction(hash)`, `routes.search(q)`, …) and `NAV_LINKS`. `<Link to={routes.X}>` / `navigate(routes.X)` instead of hand-templated strings. Single source of truth for URL shapes — encoding (`encodeURIComponent`) lives here once.                                                                                                    |
| `router/index.tsx`              | `createBrowserRouter` with the 14 spec routes hand-listed inside one `children: []` array, hung off the `AppShellStub` layout route. Each element is wrapped via a tiny `page(loader)` helper that returns `<Suspense fallback={<DetailSkeleton />}><LazyPage /></Suspense>`. Home resolves via `{ index: true }`.                                                                           |
| `router/AppShellStub.tsx`       | Placeholder layout shell (header + nav + mode toggle + `<Outlet />`) used until the real `AppShell` ships in task 0059.                                                                                                                                                                                                                                                                      |
| `router/RouteErrorBoundary.tsx` | Route-level boundary attached as `errorElement` on the layout route. Renders `NotFoundState` on 404 and `GenericErrorState` for any other throw — both inside the shell.                                                                                                                                                                                                                     |
| `pages/*.tsx`                   | One stub page per explorer route, lazy-loaded as default exports. Param validation deferred to per-page implementation tasks (0068+) — each real page validates against the API rather than duplicating regex client-side.                                                                                                                                                                   |

`VITE_API_BASE_URL` is read by Vite at build time from `web/.env.development` for
local dev. Staging and production builds receive the URL from the deployment
pipeline (CI/CDK) — no staging/production URL is committed to the repo; if the
variable is missing, `config.ts` throws a clear error at first page load.

## 9. Performance and Error Handling

- **Pagination** - all list views use cursor-based pagination backed by the block
  explorer's own database. The backend response (`PageInfo`) carries two opaque
  cursors: `next_cursor` (forward) and `prev_cursor` (backward). Both are
  direction-aware on the wire (`{dir, p}` envelope per ADR 0008 amendment, task
  0254), so every page is self-describing — Prev / Next work after refresh, deep
  link, or share without any client-side cursor history. Frontend hooks
  (`useCursorPagination` + `usePageHandlers` in `libs/ui/src/table/`) bind URL
  `?cursor=` to React Query keys; presence of `next_cursor` / `prev_cursor` in
  the response is the canonical "another page exists" signal (no `has_more`
  field).
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
