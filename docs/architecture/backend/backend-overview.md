# Stellar Block Explorer - Backend Overview

> This document expands the backend portion of
> [`technical-design-general-overview.md`](../technical-design-general-overview.md).
> It preserves the same API scope and operational assumptions, but specifies the backend
> architecture in more detail so it can later serve as input for implementation task
> planning.

---

## Table of Contents

1. [Purpose and Scope](#1-purpose-and-scope)
2. [Architectural Role](#2-architectural-role)
3. [Runtime Architecture](#3-runtime-architecture)
4. [Responsibilities and Boundaries](#4-responsibilities-and-boundaries)
5. [Module Design](#5-module-design)
6. [API Surface](#6-api-surface)
7. [Data Access and Response Model](#7-data-access-and-response-model)
8. [Caching and Performance](#8-caching-and-performance)
9. [Fault Tolerance and Operational Notes](#9-fault-tolerance-and-operational-notes)
10. [Workspace Placement and Delivery Notes](#10-workspace-placement-and-delivery-notes)

---

## 1. Purpose and Scope

The backend is the public server-side API layer of the Stellar Block Explorer. Its role is
to expose explorer data through a stable REST interface that the frontend can consume
without understanding ingestion internals, raw ledger storage layout, or XDR parsing flows.

This document covers the target design of the backend application only. It does not define
infrastructure provisioning, ingestion implementation, or the full database schema beyond
what is needed to explain backend behavior.

The current Nx workspace already reserves the backend boundary as:

- `apps/api` - application entrypoint for the public REST API
- `libs/domain` - shared explorer-domain types that may be reused by backend and frontend
- `libs/shared` - generic cross-cutting utilities with no explorer-domain vocabulary

This document describes the intended production architecture for that boundary. It is not
a description of the current implementation state, which is still skeletal.

If any statement in this file conflicts with
[`technical-design-general-overview.md`](../technical-design-general-overview.md), the
general overview document takes precedence. This file is a backend-focused refinement of
that source, not an independent redesign.

## 2. Architectural Role

The backend sits between the public clients and the block explorer's own PostgreSQL
database. It is the only supported read interface for explorer consumers.

Its job is to make indexed chain data usable:

- hide ingestion and storage details behind stable REST resources
- normalize raw indexed records into frontend-friendly responses
- enrich Soroban-specific data with readable interpretations where available
- provide unified search and consistent pagination semantics
- expose raw XDR only where advanced inspection genuinely requires it

The backend is intentionally not a chain-ingestion component and not an external-chain
proxy.

## 3. Runtime Architecture

### 3.1 Runtime Model

The backend is a NestJS application running on AWS Lambda behind API Gateway. It is a
REST API. The backend does not perform chain indexing; it reads from the block explorer's
own PostgreSQL database, which is populated by the Galexie-based ingestion pipeline.

The public explorer API serves anonymous read traffic. Browser clients do not carry API
keys; abuse controls are enforced at the ingress layer through throttling, request
validation, and AWS WAF. If API keys are introduced, they are reserved for trusted
non-browser consumers.

```
┌──────────┐    HTTPS    ┌─────────────┐              ┌──────────────────────┐
│  Client  │────────────>│ API Gateway │─────────────>│  Lambda (NestJS)     │
└──────────┘             └─────────────┘              │                      │
                                                      │  NestJS Modules:     │
                                                      │  ├─ Network ─────────┤
                                                      │  ├─ Transactions ────┤
                                                      │  ├─ Ledgers ─────────┤
                                                      │  ├─ Accounts ────────┤
                                                      │  ├─ Tokens ──────────┤
                                                      │  ├─ Contracts ───────┤
                                                      │  ├─ NFTs ────────────┤
                                                      │  ├─ Liquidity Pools ─┤
                                                      │  └─ Search ──────────┤
                                                      └──────────┬───────────┘
                                                                 │
                                                                 ▼
                                                      ┌──────────────────────┐
                                                      │  RDS PostgreSQL      │
                                                      │  (block explorer DB) │
                                                      └──────────────────────┘
```

### 3.2 Request Flow

The typical request path is:

1. client calls a public REST endpoint through API Gateway
2. API Gateway routes the request to the NestJS Lambda handler
3. the relevant module validates input and queries the explorer database
4. backend-level normalization and enrichment are applied where needed
5. the response is returned in a frontend-friendly form

### 3.3 Technology Direction

The backend implementation direction implied by the current design is:

- **NestJS** for modular API composition and transport-layer structure
- **TypeScript** for typed application code and shared contracts with workspace libraries
- **AWS Lambda** for serverless compute and on-demand scaling
- **API Gateway** for public HTTP ingress, throttling, request validation, and response
  caching
- **AWS WAF** for managed-rule abuse protection on public ingress
- **PostgreSQL** as the only source of indexed chain data served by the API
- **No XDR dependencies** — API serves pre-materialized data; raw XDR is passthrough only (per ADR 0004)

This document assumes the backend follows the implementation direction already
reflected in the general overview, including NestJS and Drizzle ORM, while keeping the API
behavior here as the primary contract to preserve.

## 4. Responsibilities and Boundaries

### 4.1 Responsibilities

The backend serves data from the block explorer's own database, adding:

- **Data normalization** - transforms raw indexed records into a consistent,
  frontend-friendly format (e.g. flattening nested fields, attaching human-readable
  operation summaries and event interpretations)
- **Soroban enrichment** - decorates contract invocations with metadata, function names,
  and structured interpretations stored at ingestion time
- **Search** - unified search across transaction hashes, account IDs, contract IDs, token
  identifiers, NFT identifiers, pool IDs, and indexed metadata using PostgreSQL full-text
  indexes
- **Raw XDR passthrough** — the `envelope_xdr`, `result_xdr`, and `result_meta_xdr` fields
  are stored verbatim; the backend returns `envelope_xdr` and `result_xdr` as opaque base64
  strings in the advanced transaction view. No server-side decode — the API serves
  pre-materialized data from the Rust ingestion pipeline (per ADR 0004)

### 4.2 What the Backend Must Not Do

The backend does **not**:

- perform live chain indexing
- call Horizon or any external chain API
- rely on a third-party explorer database
- shift protocol-specific interpretation responsibility back onto the frontend

All chain data lives in the block explorer's RDS.

### 4.3 Boundary with Other Applications

Responsibility split across the workspace should remain clear:

- `apps/indexer` and related workers own ingestion and persistence into the explorer DB
- `apps/api` owns query APIs, response shaping, search, and transport concerns
- `apps/web` consumes the REST API and should not reconstruct backend behavior client-side
- `libs/domain` may hold reusable explorer-domain types shared across the boundary
- `libs/shared` may hold generic helpers that are not explorer-specific

## 5. Module Design

The backend is best structured as resource-oriented NestJS modules matching the public API
surface.

### 5.1 Primary Modules

- `Network` - chain-level aggregate stats and freshness information
- `Transactions` - list and detail queries, filter handling, advanced/raw payload support
- `Ledgers` - ledger list/detail access and linked transaction retrieval
- `Accounts` - account summary, balances, and account-related transaction history
- `Tokens` - classic asset and Soroban token listing and detail retrieval
- `Contracts` - contract metadata, interface, invocations, and events
- `NFTs` - NFT list/detail retrieval and transfer history access
- `Liquidity Pools` - pool listing, detail, transaction history, and chart data
- `Search` - exact match and grouped result resolution across entity types

### 5.2 Cross-Cutting Backend Concerns

In addition to resource modules, the backend will need shared internal capabilities:

- request validation and query parsing
- cursor-based pagination helpers
- response serialization and error mapping
- search-query classification and exact-match resolution
- raw XDR passthrough for advanced transaction sections (no server-side decode)
- caching and freshness metadata

These are backend concerns even when their outputs are consumed by frontend pages.

## 6. API Surface

### 6.1 Base URL

**Base URL:** `https://api.soroban-explorer.com/v1`

### 6.2 Endpoint Inventory

| Resource        | Endpoint(s)                                                                                                                                             |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Network         | `GET /network/stats`                                                                                                                                    |
| Transactions    | `GET /transactions`, `GET /transactions/:hash`                                                                                                          |
| Ledgers         | `GET /ledgers`, `GET /ledgers/:sequence`                                                                                                                |
| Accounts        | `GET /accounts/:account_id`, `GET /accounts/:account_id/transactions`                                                                                   |
| Tokens          | `GET /tokens`, `GET /tokens/:id`, `GET /tokens/:id/transactions`                                                                                        |
| Contracts       | `GET /contracts/:contract_id`, `GET /contracts/:contract_id/interface`, `GET /contracts/:contract_id/invocations`, `GET /contracts/:contract_id/events` |
| NFTs            | `GET /nfts`, `GET /nfts/:id`, `GET /nfts/:id/transfers`                                                                                                 |
| Liquidity Pools | `GET /liquidity-pools`, `GET /liquidity-pools/:id`, `GET /liquidity-pools/:id/transactions`, `GET /liquidity-pools/:id/chart`                           |
| Search          | `GET /search?q=&type=transaction,contract,token,account,nft,pool`                                                                                       |

### 6.3 Resource Details

#### Network

**`GET /network/stats`** - Chain overview: current ledger sequence, TPS, total accounts,
total contracts.

This endpoint exists to support top-level explorer summary views and should remain small,
fast, and cacheable with short TTLs.

#### Transactions

**`GET /transactions`** - Paginated list. Query params: `limit`, `cursor`,
`filter[source_account]`, `filter[contract_id]`, `filter[operation_type]`.

**`GET /transactions/:hash`** - Full detail for a single transaction (supports both normal
and advanced representations):

```json
{
  "hash": "7b2a8c...",
  "ledger_sequence": 12345678,
  "source_account": "GABC...XYZ",
  "successful": true,
  "fee_charged": 100,
  "operations": [
    {
      "type": "invoke_host_function",
      "contract_id": "CCAB...DEF",
      "function_name": "swap",
      "human_readable": "Swapped 100 USDC for 95.2 XLM on Soroswap"
    }
  ],
  "operation_tree": [...],
  "events": [...],
  "envelope_xdr": "...",
  "result_xdr": "..."
}
```

Backend expectations for transactions:

- list responses must stay optimized for table-style browsing
- detail responses must support both human-readable and advanced/debugging use cases
- advanced/raw fields should be returned only where they are explicitly part of the detail contract
- transaction filters should remain backend-driven so clients do not need to post-process large result sets

#### Ledgers

**`GET /ledgers`** - Paginated list of ledgers.

**`GET /ledgers/:sequence`** - Ledger detail including transaction count and linked
transactions.

Ledger endpoints are primarily historical/browsing endpoints and should be highly cacheable
once the ledger is closed and no longer mutable.

#### Accounts

**`GET /accounts/:account_id`** - Account detail: current balances, sequence number,
and first/last seen ledger.

**`GET /accounts/:account_id/transactions`** - Paginated transactions involving this
account.

The current account scope is intentionally limited to:

- account summary
- balances
- recent transaction history

This keeps account support aligned with the currently documented product scope and avoids
expanding the backend contract beyond what the frontend is expected to show.

#### Tokens

**`GET /tokens`** - Paginated list of tokens (classic assets + Soroban token contracts).
Query params: `limit`, `cursor`, `filter[type]` (classic/sac/soroban), `filter[code]`.

**`GET /tokens/:id`** - Token detail: asset code, issuer/contract, type, supply, holder
count, metadata.

**`GET /tokens/:id/transactions`** - Paginated transactions involving this token.

The backend must preserve the distinction between classic assets and contract-based tokens
while still serving both through a unified explorer API.

#### Contracts

**`GET /contracts/:contract_id`** - Contract metadata, deployer, WASM hash, stats.

**`GET /contracts/:contract_id/interface`** - Public function signatures (names, parameter
types, return types).

**`GET /contracts/:contract_id/invocations`** - Paginated list of contract invocations.

**`GET /contracts/:contract_id/events`** - Paginated list of contract events.

Contract endpoints are the most Soroban-specific part of the API and should remain the main
place where indexed contract metadata and decoded usage history are exposed.

#### NFTs

**`GET /nfts`** - Paginated list of NFTs. Query params: `limit`, `cursor`,
`filter[collection]`, `filter[contract_id]`.

**`GET /nfts/:id`** - NFT detail: name, token ID, collection, contract, owner, metadata,
media URL.

**`GET /nfts/:id/transfers`** - Transfer history for a single NFT.

NFT responses should tolerate sparse metadata because the ecosystem and available metadata
quality may vary significantly.

#### Liquidity Pools

**`GET /liquidity-pools`** - Paginated list of pools. Query params: `limit`, `cursor`,
`filter[assets]`, `filter[min_tvl]`.

**`GET /liquidity-pools/:id`** - Pool detail: asset pair, fee, reserves, total shares, TVL.

**`GET /liquidity-pools/:id/transactions`** - Deposits, withdrawals, and trades for this
pool.

**`GET /liquidity-pools/:id/chart`** - Time-series data for TVL, volume, and fee revenue.
Query params: `interval` (1h/1d/1w), `from`, `to`.

These endpoints combine factual current-state reads with historical aggregate reads, so the
backend should keep raw pool state and chart-series generation concerns clearly separated.

#### Search

**`GET /search?q=&type=transaction,contract,token,account,nft,pool`** - Generic search
across all entity types. Uses prefix/exact matching on hashes, account IDs, contract IDs,
asset codes, pool IDs, and NFT identifiers. Full-text search on metadata via
`tsvector`/`tsquery` and GIN indexes where entity metadata is indexed.

Search is not just a DB query wrapper. It is an API behavior surface that must:

- classify likely query types
- support exact-match redirect behavior in the consuming frontend
- return grouped broad-search results for ambiguous inputs

## 7. Data Access and Response Model

### 7.1 Source of Data

All backend reads come from the block explorer's own PostgreSQL database. The API should
never depend on live calls to Horizon, Soroban RPC, or third-party indexers for core
resource responses.

### 7.2 Response Shaping

The backend should expose read models designed for explorer use, not raw storage rows.
That means:

- flattening or restructuring nested data where that improves client usability
- attaching human-readable labels produced upstream during ingestion or interpretation
- keeping raw protocol payloads available only for advanced/detail use cases
- preserving stable identifier fields needed for linking across pages

### 7.3 Pagination Semantics

Collection endpoints should use cursor-based pagination consistently.

API-level expectations:

- cursors are opaque to clients
- pagination should not depend on expensive total counts
- ordering should remain deterministic for stable browsing
- list filters must be applied in the backend query layer, not in the client

### 7.4 Normal vs Advanced Transaction Data

Transaction detail is the clearest example of a dual-mode backend contract:

- the normal view is centered on interpreted operations, call trees, and readable summaries
- the advanced view includes raw parameters, raw event payloads, and raw XDR where needed

The backend should treat these as two representations over the same transaction resource,
not as two unrelated endpoints with diverging semantics.

## 8. Caching and Performance

### 8.1 Caching Strategy

Caching operates at two levels:

- **API Gateway response caching** - responses for immutable data (historical
  transactions, closed ledgers) are cached with long TTLs at the API ingress layer. Mutable
  data (recent transactions, network stats) uses short TTLs (5-15 seconds). CloudFront is
  reserved for static frontend/document delivery in the initial topology.
- **Backend in-memory caching** - frequently accessed reference data (contract metadata,
  network stats) is cached in the Lambda execution environment with TTLs of 30-60 seconds
  to reduce database round-trips.

### 8.2 Performance Expectations

The backend should optimize for explorer-style read traffic:

- many small GET requests from route-driven navigation
- repeated detail-page lookups for well-known identifiers
- list browsing with filters and cursor pagination
- bursty traffic on recently closed ledgers and popular contract/token pages

The API should avoid pushing expensive post-processing to the client when that would create
inconsistent results or duplicated logic across screens.

## 9. Fault Tolerance and Operational Notes

### 9.1 Fault Tolerance

- **Ingestion lag** - if the Galexie pipeline falls behind, the API continues serving
  data from the database with a freshness indicator showing the highest indexed ledger
  sequence. A CloudWatch alarm fires at >60 s lag.
- **Lambda cold starts** - mitigated via ARM/Graviton2 runtime and provisioned concurrency
  at higher traffic tiers.
- **Database connection pooling** - RDS Proxy manages connection pools to prevent
  exhaustion under burst traffic.

### 9.2 Operational Boundary

The backend should degrade gracefully when upstream ingestion is delayed. It should serve
what is already indexed and make freshness visible rather than failing simply because the
network tip has advanced.

It should also remain operationally simple:

- read-only with respect to chain data
- no dependence on external chain APIs for core functionality
- clean separation between transport concerns, query logic, and response shaping

## 10. Workspace Placement and Delivery Notes

The workspace currently provides the structural backend boundary (`apps/api`) but not the
final NestJS runtime implementation yet. That is consistent with the repository README and
current bootstrap status.

Expected code placement:

- `apps/api` for application bootstrap, route wiring, NestJS modules, and runtime integrations
- `libs/domain` for reusable explorer-domain types and value objects shared with other apps
- `libs/shared` for generic helpers that are not specific to explorer business concepts

This document should be treated as the detailed reference for future backend implementation
planning, with
[`technical-design-general-overview.md`](../technical-design-general-overview.md) remaining
the primary source of truth.
