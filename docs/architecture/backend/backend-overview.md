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

The target workspace structure (per ADR 0005, tasks 0094/0095) reserves the backend boundary as:

- `crates/api` - application entrypoint for the public REST API (Rust/axum)
- `crates/domain` - shared explorer-domain types used by backend crates

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
- provide unified search and consistent pagination semantics
- expose raw XDR only where advanced inspection genuinely requires it

The backend is intentionally not a chain-ingestion component and not an external-chain
proxy.

## 3. Runtime Architecture

### 3.1 Runtime Model

The backend is a Rust application (axum + sqlx) running on AWS Lambda behind API Gateway. It is a
REST API. The backend does not perform chain indexing; it reads from the block explorer's
own PostgreSQL database, which is populated by the Galexie-based ingestion pipeline.

The public explorer API serves anonymous read traffic. Browser clients do not carry API
keys; abuse controls are enforced at the ingress layer through throttling, request
validation, and AWS WAF. If API keys are introduced, they are reserved for trusted
non-browser consumers.

```
┌──────────┐    HTTPS    ┌─────────────┐              ┌──────────────────────┐
│  Client  │────────────>│ API Gateway │─────────────>│  Lambda (Rust/axum)  │
└──────────┘             └─────────────┘              │                      │
                                                      │  axum Modules:       │
                                                      │  ├─ Network ─────────┤
                                                      │  ├─ Transactions ────┤
                                                      │  ├─ Ledgers ─────────┤
                                                      │  ├─ Accounts ────────┤
                                                      │  ├─ Assets ──────────┤
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
2. API Gateway routes the request to the Rust/axum Lambda handler
3. the relevant module validates input and queries the explorer database
4. backend-level normalization and enrichment are applied where needed
5. the response is returned in a frontend-friendly form

### 3.3 Technology Direction

The backend implementation direction implied by the current design is:

- **axum** for modular API composition and transport-layer structure (per ADR 0005)
- **Rust** for typed application code with compile-time safety
- **sqlx** for compile-time checked database queries (per ADR 0005)
- **utoipa** for OpenAPI spec generation (per ADR 0005)
- **AWS Lambda** for serverless compute and on-demand scaling (via cargo-lambda)
- **API Gateway** for public HTTP ingress, throttling, request validation, and response
  caching
- **AWS WAF** for managed-rule abuse protection on public ingress
- **PostgreSQL** as the only source of indexed chain data served by the API
- **No XDR dependencies** — API serves pre-materialized data; raw XDR is passthrough only (per ADR 0004)

This document assumes the backend follows the implementation direction already
reflected in the general overview, including axum, sqlx, and utoipa (per ADR 0005), while keeping the API
behavior here as the primary contract to preserve.

## 4. Responsibilities and Boundaries

### 4.1 Responsibilities

The backend serves data from the block explorer's own database, adding:

- **Data normalization** - transforms raw indexed records into a consistent,
  frontend-friendly format (e.g. flattening nested fields, attaching human-readable
  operation summaries)
- **Soroban enrichment** - decorates contract invocations with metadata and function names
  stored at ingestion time
- **Search** - unified search across transaction hashes, account IDs, contract IDs, token
  identifiers, NFT identifiers, pool IDs, and indexed metadata using PostgreSQL full-text
  indexes
- **Read-time XDR fetch for heavy-field endpoints** — per
  [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md),
  the backend does **not** store raw envelope / result / result-meta XDR on
  `transactions`, and per
  [ADR 0033](../../../lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md) /
  [ADR 0034](../../../lore/2-adrs/0034_soroban-invocations-appearances-read-time-detail.md)
  it does not store decoded events / invocation-tree nodes either. For E3
  `/transactions/:hash` (full envelope + parsed invocation tree),
  E13 `/contracts/:id/invocations` (per-node function name / args / return value),
  and E14 `/contracts/:id/events` (full event detail) the API fetches the
  corresponding `.xdr.zst` from the public Stellar ledger archive, decompresses
  it, parses it with the shared `crates/xdr-parser` crate, and merges the
  decoded payload into the response. List endpoints never call the archive and
  answer from typed summary columns + appearance indexes only
- **Surrogate-key resolution** — every StrKey that enters a route parameter
  (`G...`, `C...`) is resolved to the `BIGINT` surrogate via the relevant
  `UNIQUE` index at the request boundary
  ([ADR 0026](../../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md),
  [ADR 0030](../../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md));
  every StrKey in a response comes from a join back to `accounts.account_id`
  or `soroban_contracts.contract_id`. The public API shape is unchanged

### 4.2 What the Backend Must Not Do

The backend does **not**:

- perform live chain indexing
- call Horizon or any private chain API
- rely on a third-party explorer database

Backend dependencies at runtime: (1) the explorer's own RDS for every
partition-pruned read, (2) the public Stellar ledger archive for read-time
XDR expansion on E3 / E14.

### 4.3 Boundary with Other Applications

Responsibility split across the workspace should remain clear:

- `crates/indexer` and related workers own ingestion and persistence into the explorer DB
- `crates/api` owns query APIs, response shaping, search, and transport concerns
- `web` consumes the REST API and should not reconstruct backend behavior client-side
- `crates/domain` holds reusable explorer-domain types shared across backend crates

## 5. Module Design

The backend is best structured as resource-oriented axum route modules matching the public API
surface.

### 5.1 Primary Modules

- `Network` - chain-level aggregate stats and freshness information
- `Transactions` - list and detail queries, filter handling, advanced/raw payload support
- `Ledgers` - ledger list/detail access and linked transaction retrieval
- `Accounts` - account summary, balances, and account-related transaction history
- `Assets` - classic and Soroban-native asset listing and detail retrieval
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

| Resource        | Endpoint(s)                                                                                                                                                            |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Network         | `GET /network/stats`                                                                                                                                                   |
| Transactions    | `GET /transactions`, `GET /transactions/:hash`                                                                                                                         |
| Ledgers         | `GET /ledgers`, `GET /ledgers/:sequence`                                                                                                                               |
| Accounts        | `GET /accounts/:account_id`, `GET /accounts/:account_id/transactions`                                                                                                  |
| Assets          | `GET /assets`, `GET /assets/:id`, `GET /assets/:id/transactions`                                                                                                       |
| Contracts       | `GET /contracts/:contract_id`, `GET /contracts/:contract_id/interface`, `GET /contracts/:contract_id/invocations`, `GET /contracts/:contract_id/events`                |
| NFTs            | `GET /nfts`, `GET /nfts/:id`, `GET /nfts/:id/transfers`                                                                                                                |
| Liquidity Pools | `GET /liquidity-pools`, `GET /liquidity-pools/:id`, `GET /liquidity-pools/:id/transactions`, `GET /liquidity-pools/:id/chart`, `GET /liquidity-pools/:id/participants` |
| Search          | `GET /search?q=&type=transaction,contract,asset,account,nft,pool`                                                                                                      |

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
      "function_name": "swap"
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

#### Assets

**`GET /assets`** - Paginated list of assets (native XLM, classic credit assets, SACs, and Soroban-native assets).
Query params: `limit`, `cursor`, `filter[type]` (native/classic_credit/sac/soroban), `filter[code]`.

**`GET /assets/:id`** - Asset detail: asset code, issuer/contract, type, supply, holder
count, metadata.

**`GET /assets/:id/transactions`** - Paginated transactions involving this asset.

The backend must preserve the distinction between native, classic credit, SAC, and
Soroban-native assets while still serving all through a unified explorer API.

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
`filter[collection]` (exact match), `filter[contract_id]` (C-StrKey), `filter[name]`
(substring; rejects `%`/`_` literals — canonical SQL `15_get_nfts_list.sql`).

**`GET /nfts/:id`** - NFT detail: name, token ID, collection, contract, owner, metadata,
media URL.

**`GET /nfts/:id/transfers`** - Transfer history for a single NFT.

NFT responses should tolerate sparse metadata because the ecosystem and available metadata
quality may vary significantly.

#### Liquidity Pools

**`GET /liquidity-pools`** - Paginated list of pools. Query params: `limit`, `cursor`,
`filter[asset_a_code]`, `filter[asset_a_issuer]` (G-StrKey), `filter[asset_b_code]`,
`filter[asset_b_issuer]` (G-StrKey), `filter[min_tvl]` (decimal). Per-leg
`(code, issuer)` must be supplied paired or both omitted (classic identity).
Filter semantics in canonical SQL `18_get_liquidity_pools_list.sql`.

**`GET /liquidity-pools/:id`** - Pool detail: asset pair, fee, reserves, total shares, TVL.
Dynamic fields come from the latest snapshot row; clients that care about
freshness read `latest_snapshot_at` in the response.

**`GET /liquidity-pools/:id/transactions`** - Deposits, withdrawals, and trades for this
pool.

**`GET /liquidity-pools/:id/chart`** - Time-series data for TVL, volume, and fee revenue.
Query params (all optional, sensible defaults): `interval` (`1h`/`1d`/`1w`,
default `1d`), `from` (ISO 8601, default `to` minus interval-appropriate
window — `1h→7d`, `1d→90d`, `1w→104w`), `to` (ISO 8601, default `now()`,
exclusive upper bound). `from < to` enforced; bucket count capped to keep
aggregation bounded. Bucket aggregation policy in canonical SQL
`21_get_liquidity_pools_chart.sql`.

**`GET /liquidity-pools/:id/participants`** - Paginated list of liquidity providers
with their share size, share percentage of the pool, first deposit ledger, and last
update ledger. Powers the "Pool participants" table on the LP detail page
(frontend §6.14). Backed by `lp_positions` (ADR 0037 §16). Added during task 0167
to close a doc-drift gap between the frontend page and the original endpoint
inventory.

These endpoints combine factual current-state reads with historical aggregate reads, so the
backend should keep raw pool state and chart-series generation concerns clearly separated.

#### Search

**`GET /search?q=&type=transaction,contract,asset,account,nft,pool`** - Generic search
across all entity types. Uses prefix/exact matching on hashes, account IDs, contract IDs,
asset codes, pool IDs, and NFT identifiers. Full-text search on metadata via
`tsvector`/`tsquery` and GIN indexes where entity metadata is indexed.

Search is not just a DB query wrapper. It is an API behavior surface that must:

- classify likely query types
- support exact-match redirect behavior in the consuming frontend
- return grouped broad-search results for ambiguous inputs

## 7. Data Access and Response Model

### 7.1 Source of Data

List endpoints and all partition-pruned reads come from the block explorer's own
PostgreSQL database. Heavy-field detail endpoints (E3 `/transactions/:hash`,
E14 `/contracts/:id/events`) additionally fetch raw `.xdr.zst` from the **public
Stellar ledger archive** and re-parse it at request time per
[ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md).
The API does not depend on Horizon, Soroban RPC, or third-party indexers for any
response.

### 7.2 Response Shaping

The backend should expose read models designed for explorer use, not raw storage rows.
That means:

- flattening or restructuring nested data where that improves client usability
- attaching human-readable labels produced upstream during ingestion
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

- the normal view is centered on decoded operations and call trees
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
  to reduce database round-trips. The contract-detail cache is implemented in
  `crates/api/src/contracts/cache.rs` (`ContractMetadataCache`, 45 s TTL,
  lazy eviction); it is keyed by contract StrKey and shared across
  handler invocations on the same warm Lambda container.

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
- **Lambda cold starts** - mitigated via Rust's fast startup on ARM/Graviton2 and provisioned concurrency
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

The target workspace will provide the structural backend boundary (`crates/api`, per tasks 0094/0095) but the
Rust/axum runtime implementation is not yet in place. That is consistent with the repository README and
current bootstrap status.

Expected code placement:

- `crates/api` for application bootstrap, route wiring, axum modules, and runtime integrations
- `crates/domain` for reusable explorer-domain types and value objects shared across backend crates

This document should be treated as the detailed reference for future backend implementation
planning, with
[`technical-design-general-overview.md`](../technical-design-general-overview.md) remaining
the primary source of truth.
