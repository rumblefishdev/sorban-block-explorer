# Stellar Block Explorer — Technical Design (Post-Review)

> This document supersedes `soroban-first-block-explorer.md`. It incorporates changes required by reviewer
> feedback: the indexing layer is rebuilt around self-hosted Galexie (direct ledger
> processing) replacing the deprecated Horizon API, the block explorer now owns its own
> database, and component ownership is explicitly demarcated.

A production-grade, Soroban-first block explorer for the Stellar network. The system
prioritizes **human-readable transaction display** and first-class Soroban smart contract
support. The frontend communicates exclusively with a custom Rust/axum REST API (per ADR 0005), which sources
chain data from the block explorer's own ClickHouse store — populated by a Galexie-based
ingestion pipeline that processes `LedgerCloseMeta` XDR directly from the Stellar network.

> **Store status.** ClickHouse (self-hosted, Hetzner + mTLS) is the sole production
> datastore per [ADR 0047](../../lore/2-adrs/0047_clickhouse-primary-api-datastore.md);
> Postgres/sqlx were fully removed from the codebase in task 0244.
>
> **The production RDS PostgreSQL described in earlier revisions of this document was
> never deployed.** It was the original design, superseded before it shipped. Only a
> _staging_ RDS ever ran, in `us-east-1`, and it was destroyed with the rest of the
> staging footprint on 2026-05-21
> ([task 0249](../../lore/1-tasks/archive/0249_FEATURE_destroy-aws-infra-us-east-1.md));
> production stacks were never deployed there at all — `validateConfig` blocked on
> `hostedZoneId: "CHANGE_ME"`. Consequently
> [task 0239](../../lore/1-tasks/archive/0239_FEATURE_aws-side-cutover-mtls-to-hetzner.md)
> Phase 6 ("decommission") closed **vacuously** — there was nothing running to tear
> down — and the RDS/bastion CDK stacks were moved to `.trash/`.
>
> Read any remaining RDS mention below as **superseded design intent, not deployment
> history**. Task 0248 swept the present-tense claims in §3, §7 and the cost tables
> onto the ClickHouse-on-Hetzner topology that actually runs.

---

## Table of Contents

1. [Frontend](#1-frontend)
2. [Backend](#2-backend)
3. [Infrastructure](#3-infrastructure)
4. [Indexing Pipeline (Galexie)](#4-indexing-pipeline-galexie)
5. [XDR Parsing](#5-xdr-parsing)
6. [Database Schema](#6-database-schema)
7. [Estimates](#7-estimates)

---

## 1. Frontend

### 1.1 Goals

- **Human-readable format** — Show exactly what occurred in each transaction. Users should
  understand payments, DEX operations, and Soroban contract calls without decoding XDR or
  raw operation codes.
- **Classic + Soroban** — Support both classic Stellar operations (payments, offers, path
  payments, etc.) and Soroban operations (invoke host function, contract events, token swaps).

### 1.2 Architecture

The frontend is a React application served via CloudFront CDN. It consumes the backend
REST API with polling-based updates for new transactions and events.

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
               │  /nfts/:contract_id/:token_id                  ── GET /nfts/:contract_id/:token_id ───────────┤   │
               │  /liquidity-pools           ── GET /liquidity-pools ────┤   │
               │  /liquidity-pools/:strkey       ── GET /liquidity-pools/:strkey ┤   │
               │  /search?q=                 ── GET /search ─────────────┘   │
               │                                         │                   │
               └─────────────────────────────────────────┼───────────────────┘
                                                         │
                                                         ▼
                                                 ┌──────────────┐
                                                 │   REST API   │
                                                 └──────────────┘
```

### 1.3 Routes and Pages

| Route                          | Page            | Primary API Endpoint(s)                                                     |
| ------------------------------ | --------------- | --------------------------------------------------------------------------- |
| `/`                            | Home            | `GET /network/stats`, `GET /transactions?limit=10`, `GET /ledgers?limit=10` |
| `/transactions`                | Transactions    | `GET /transactions`                                                         |
| `/transactions/:hash`          | Transaction     | `GET /transactions/:hash`                                                   |
| `/ledgers`                     | Ledgers         | `GET /ledgers`                                                              |
| `/ledgers/:sequence`           | Ledger          | `GET /ledgers/:sequence`                                                    |
| `/accounts/:accountId`         | Account         | `GET /accounts/:account_id`, `GET /accounts/:account_id/transactions`       |
| `/assets`                      | Assets          | `GET /assets`                                                               |
| `/assets/:id`                  | Asset           | `GET /assets/:id`, `GET /assets/:id/transactions`                           |
| `/contracts/:contractId`       | Contract        | `GET /contracts/:contract_id`, `GET /contracts/:contract_id/interface`      |
| `/nfts`                        | NFTs            | `GET /nfts`                                                                 |
| `/nfts/:contract_id/:token_id` | NFT             | `GET /nfts/:contract_id/:token_id`                                          |
| `/liquidity-pools`             | Liquidity Pools | `GET /liquidity-pools`                                                      |
| `/liquidity-pools/:strkey`     | Liquidity Pool  | `GET /liquidity-pools/:strkey`                                              |
| `/search?q=`                   | Search Results  | `GET /search`                                                               |

#### Home (`/`)

Entry point and chain overview. Provides at-a-glance state of the Stellar network and
quick access to exploration.

- Global search bar — accepts transaction hashes, contract IDs, token codes, account IDs,
  ledger sequences
- Latest transactions table — hash (truncated), source account, operation type, status
  badge, timestamp
- Latest ledgers table — sequence, closed_at, transaction count
- Chain overview — current ledger sequence, transactions per second, total accounts,
  total contracts

#### Transactions (`/transactions`)

Paginated, filterable table of all indexed transactions. Default sort: most recent first.

- Transaction table — hash, ledger sequence, source account, operation type, status badge
  (success/failed), fee, timestamp
- Filters — source account, contract ID, operation type
- Cursor-based pagination controls

#### Transaction (`/transactions/:hash`)

Both modes display the same base transaction details:

- Transaction hash (full, copyable), status badge (success/failed), ledger sequence
  (link), timestamp
- Fee charged (XLM + stroops), source account (link), memo (type + content)
- Signatures — signer, weight, signature hex

Two display modes toggle how **operations** are presented:

- **Normal mode** — graph/tree representation of the transaction's operation flow.
  Visually shows the relationships between source account, operations, and affected
  accounts/contracts. Each node in the tree displays a human-readable summary (e.g.
  "Sent 1,250 USDC to GD2M…K8J1", "Swapped 100 USDC for 95.2 XLM on Soroswap"). Soroban
  invocations render as a nested call tree showing the contract-to-contract hierarchy.
  Designed for general users exploring transactions.

- **Advanced mode** — targeted at developers and experienced users. Shows per-operation
  raw parameters, full argument values, operation IDs, and return values. Includes events
  emitted (type, topics, raw data), diagnostic events, and collapsible raw XDR sections
  (`envelope_xdr`, `result_xdr`, `result_meta_xdr`). All values are shown in their
  original format without simplification.

#### Ledgers (`/ledgers`)

Paginated table of all ledgers. Default sort: most recent first.

- Ledger table — sequence, hash (truncated), closed_at, protocol version, transaction count
- Cursor-based pagination controls

#### Ledger (`/ledgers/:sequence`)

- Ledger summary — sequence, hash, closed_at, protocol version, transaction count, base fee
- Transactions in ledger — paginated table of all transactions in this ledger
- Previous / next ledger navigation

#### Account (`/accounts/:accountId`)

Account detail view for a Stellar account.

- Account summary — account ID (full, copyable), sequence number, first seen ledger, last seen ledger
- Balances — native XLM balance and credit asset balances
- Recent transactions — paginated table of transactions involving this account

#### Assets (`/assets`)

List of all known assets (native XLM, classic credit assets, SACs, and Soroban-native assets).

- Asset table — asset code, issuer / contract ID, type (native / classic credit / SAC / Soroban), total
  supply, holder count
- Filters — type (native, classic_credit, SAC, Soroban), asset code search
- Cursor-based pagination controls

#### Asset (`/assets/:id`)

Single asset detail view.

- Asset summary — asset code, issuer or contract ID (copyable), type badge, total supply,
  holder count, deployed at ledger (if Soroban)
- Metadata — name, description, icon (if available), domain/home page
- Latest transactions — paginated table of recent transactions involving this asset

#### Contract (`/contracts/:contractId`)

Contract details and interface.

- Contract summary — contract ID (full, copyable), deployer account (link), deployed at
  ledger (link), WASM hash, SAC badge if applicable
- Contract interface — list of public functions with parameter names and types, allowing
  users to understand the contract's API without reading source code
- Invocations tab — recent invocations table (function name, caller account, status,
  ledger, timestamp)
- Events tab — recent events table (event type, topics, data, ledger)
- Stats — total invocations count, unique callers

#### NFTs (`/nfts`)

List of NFTs on the Stellar network (Soroban-based NFT contracts).

- NFT table — name/identifier, collection name, contract ID, owner, preview image
- Filters — collection, contract ID
- Cursor-based pagination controls

#### NFT (`/nfts/:contract_id/:token_id`)

Single NFT overview.

- NFT summary — name, identifier/token ID, collection name, contract ID (link), owner
  account (link)
- Media preview — image, video, or other media associated with the NFT
- Metadata — full attribute list (traits, properties)
- Transfer history — table of ownership changes

#### Liquidity Pools (`/liquidity-pools`)

Paginated table of all liquidity pools.

- Pool table — pool ID (truncated), the pool's legs (e.g. `XLM / USDC`, or three
  for a Soroban stable pool), total shares, reserves per leg, fee percentage,
  kind badge
- Filters — asset code or pair, pool kind (classic / Soroban); minimum TVL is
  rejected with a 400 (computed at read, so it cannot filter page membership)
- Cursor-based pagination controls

#### Liquidity Pool (`/liquidity-pools/:strkey`)

- Pool summary — pool ID (full, copyable), the pool's legs, fee percentage,
  total shares, reserves per leg
- Charts — TVL over time, volume over time, fee revenue
- Pool participants — table of liquidity providers and their share
- Recent transactions — deposits, withdrawals, and trades involving this pool

#### Search Results (`/search?q=`)

Generic search across all entity types. For exact matches (transaction hash, contract ID,
account ID), redirects directly to the detail page. Otherwise displays grouped results.

- Search input — pre-filled with current query, allows refinement
- Results grouped by type — transactions, contracts, assets, accounts, NFTs, liquidity
  pools (with type headers and counts)
- Each result row — identifier (linked), type badge, brief context
- Empty state — "No results found" with suggestions

### 1.4 Shared UI Elements

Present across all pages:

- **Header** — logo, global search bar, network indicator (mainnet/testnet)
- **Navigation** — links to home, transactions, ledgers, assets, contracts, NFTs,
  liquidity pools
- **Linked identifiers** — all hashes, account IDs, contract IDs, token IDs, pool IDs,
  and ledger sequences are clickable links to their respective detail pages
- **Copy buttons** — on all full-length identifiers
- **Relative timestamps** — "2 min ago" with full timestamp on hover
- **Polling indicator** — shows when data was last refreshed

### 1.5 Performance and Error Handling

- **Pagination** — all list views use cursor-based pagination backed by the block
  explorer's own database
- **Loading states** — skeleton loaders for all data-dependent sections; spinner for search
- **Error states** — clear error messages for network failures, 404s (unknown
  hash/account), and rate limit responses; retry affordances where appropriate
- **Caching** — the frontend relies on backend-level caching (CloudFront, API Gateway)
  rather than local state caching to ensure data freshness

---

## 2. Backend

### 2.1 Architecture

The backend is a Rust application (axum + utoipa, per ADR 0005) running on AWS Lambda behind API Gateway. It is a
REST API. The backend does not perform chain indexing; it reads from the block explorer's
own ClickHouse store, which is populated by the Galexie-based ingestion pipeline.

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
                                                      │  ClickHouse (mTLS)   │
                                                      │  Hetzner ch-prod-01  │
                                                      └──────────────────────┘
```

### 2.2 API Responsibilities and Boundaries

The backend serves data from the block explorer's own database, adding:

- **Data normalization** — transforms raw indexed records into a consistent,
  frontend-friendly format (e.g. flattening nested fields, attaching human-readable
  operation summaries)
- **Soroban enrichment** — decorates contract invocations with metadata and function names
  stored at ingestion time
- **Search** — unified search across transaction hashes, account IDs, contract IDs, token
  identifiers, NFT identifiers, pool IDs, and indexed metadata via ClickHouse
  classification-gated per-bucket lookups (task 0318)
- **Raw XDR on demand** — for heavy-field endpoints (E3 `/transactions/:hash`,
  E14 `/contracts/:id/events`) the backend fetches the corresponding `.xdr.zst`
  from the public Stellar ledger archive, decompresses and parses it, and merges
  the decoded envelope / result / result-meta / events / invocation tree into the
  response. Per
  [ADR 0029](../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md),
  the DB does not store raw XDR — typed summary columns on `transactions` /
  `operations_appearances` are sufficient for list endpoints, and read-time archive fetch
  powers the detail endpoints

The backend does **not** call Horizon or any private chain API. Its dependencies are
(1) the explorer's own ClickHouse store for every partition-pruned read and (2) the public Stellar
ledger archive for read-time XDR expansion on E3 / E14.

### 2.3 Endpoints

**Base URL:** `https://api.soroban-explorer.com/v1`

#### Network

**`GET /network/stats`** — Chain overview: current ledger sequence, TPS, total accounts,
total contracts.

#### Transactions

**`GET /transactions`** — Paginated list. Query params: `limit`, `cursor`,
`filter[source_account]`, `filter[contract_id]`, `filter[operation_type]`.

**`GET /transactions/:hash`** — Full detail for a single transaction (supports both normal
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

#### Ledgers

**`GET /ledgers`** — Paginated list of ledgers.

**`GET /ledgers/:sequence`** — Ledger detail including transaction count and linked
transactions.

#### Accounts

**`GET /accounts/:account_id`** — Account detail: current balances, sequence number,
and first/last seen ledger.

**`GET /accounts/:account_id/transactions`** — Paginated transactions involving this
account.

#### Assets

**`GET /assets`** — Paginated list of assets (native XLM, classic credit assets, SACs, Soroban-native assets).
Query params: `limit`, `cursor`, `filter[type]` (native/classic_credit/sac/soroban), `filter[code]`.

**`GET /assets/:id`** — Asset detail: asset code, issuer/contract, type, supply, holder
count, metadata.

**`GET /assets/:id/transactions`** — Paginated transactions involving this asset.

#### Contracts

**`GET /contracts/:contract_id`** — Contract metadata, deployer, WASM hash, stats.

**`GET /contracts/:contract_id/interface`** — Public function signatures (names, parameter
types, return types).

**`GET /contracts/:contract_id/invocations`** — Paginated list of contract invocations.

**`GET /contracts/:contract_id/events`** — Paginated list of contract events.

#### NFTs

**`GET /nfts`** — Paginated list of NFTs. Query params: `limit`, `cursor`,
`filter[collection]`, `filter[contract_id]`.

**`GET /nfts/:contract_id/:token_id`** — NFT detail: name, token ID, collection, contract, owner, metadata,
media URL.

**`GET /nfts/:contract_id/:token_id/transfers`** — Transfer history for a single NFT.

#### Liquidity Pools

**`GET /liquidity-pools`** — Paginated list of pools. Query params: `limit`, `cursor`,
`filter[asset_code]`, `filter[pool_kind]`.

**`GET /liquidity-pools/:strkey`** — Pool detail: legs, kind, fee, reserves, total shares, TVL.

**`GET /liquidity-pools/:strkey/transactions`** — Deposits, withdrawals, and trades for this
pool.

**`GET /liquidity-pools/:strkey/chart`** — Time-series data for TVL, volume, and fee revenue,
all USD computed at read (see §6.11 and [ADR 0053](../../lore/2-adrs/0053_fast-change-offchain-compute-at-read.md)).
Query params: `interval` (1h/1d/1w), `from`, `to`. The 2026-06 "TVL only" launch
cut was retired 2026-08-04 (task 0199): `gross_volume_a` is backfilled (0266)
and the prices views are live in-cluster, so all three series ship together.

#### Search

**`GET /search?q=&type=transaction,contract,asset,account,nft,pool`** — Generic search
across all entity types. Uses prefix/exact matching on hashes, account IDs, contract IDs,
asset codes, pool IDs, and NFT identifiers. Full-text search on metadata via
`tsvector`/`tsquery` and GIN indexes where entity metadata is indexed.

### 2.4 Caching Strategy

Caching operates at two levels:

- **Browser caching via `Cache-Control`** — every public endpoint advertises a
  per-endpoint TTL (task 0055: 10 s / 60 s / 300 s tiers, `no-store` on search
  and on every non-2xx). The consumer today is the **user's browser**. The API
  Gateway stage cache that was once planned to consume these headers is **not
  enabled and stays off by decision** (2026-08-14, task 0455) — see
  [`backend/api-gateway-cache-spec.md`](./backend/api-gateway-cache-spec.md)
  for the measurement, the rationale and the return condition. CloudFront
  serves static frontend assets and is not a cache layer for API responses.
- **Backend in-memory caching** — frequently accessed reference data (contract metadata,
  network stats) is cached in the Lambda execution environment with TTLs of 30–60 seconds
  to reduce database round-trips. All in-process caches are built on the
  `moka` crate via a shared helper (`crates/api/src/cache.rs`); see
  `docs/architecture/backend/backend-overview.md` §8.1 for the concrete
  caches and bounds.

### 2.5 Fault Tolerance

- **Ingestion lag** — if the Galexie pipeline falls behind, the API continues serving
  data from the database with a freshness indicator showing the highest indexed ledger
  sequence. A CloudWatch alarm fires when no new ledger file lands in S3 (zero
  doorbell messages on the ingest queue) for the configured lag window
  (`galexieLagMinutes`, 5 min in production), with missing data treated as
  breaching — a silent stop is an alarm state, not "no data".
- **Lambda cold starts** — mitigated via ARM/Graviton2 runtime and provisioned concurrency
  at higher traffic tiers.
- **Database connections** — no proxy tier. Each Lambda constructs one
  `clickhouse::Client` at cold start and clones it per request; the client is
  HTTP-based and cheap to clone, so there is no pool to exhaust.

---

## 3. Infrastructure

### 3.1 System Architecture

```
┌───────────────────────────────────────────────────────────────────────────────┐
│                             SYSTEM ARCHITECTURE                               │
├───────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  STELLAR NETWORK          INGESTION (GALEXIE)           STORAGE               │
│  ┌──────────────────┐    ┌──────────────────────┐    ┌─────────────────────┐  │
│  │ Stellar Network  │    │ Galexie (ECS Fargate) │    │ S3                  │  │
│  │ Peers (Captive   │───>│ Continuously running  │───>│ LedgerCloseMeta XDR │  │
│  │ Core)            │    │ ~1 file per ledger    │    │ (transient)         │  │
│  └──────────────────┘    └──────────────────────┘    └──────────┬──────────┘  │
│                                                                 │             │
│  PROCESSING                                                     │ S3 PutObject│
│  ┌──────────────────────────────────────────────────────┐       │             │
│  │ Lambda — Ledger Processor (event-driven, per file)   │<──────┘             │
│  │ Parses XDR → ledgers, txs, ops, accounts, events,    │                     │
│  │ contracts                                             │                     │
│  └──────────────────────────┬───────────────────────────┘                     │
│                             │                                                 │
│  ┌──────────────────────────▼───────────────────────────┐                     │
│  │ ClickHouse — Hetzner ch-prod-01, reached over mTLS   │                     │
│  │ ledgers · transactions · operations_appearances      │                     │
│  │ accounts · transaction_participants · tx_hash_index  │                     │
│  │ soroban_contracts · wasm_interface_metadata · assets │                     │
│  │ soroban_events_appearances · soroban_invocations_…   │                     │
│  │ nfts · nft_ownership · liquidity_pools · lp_…        │                     │
│  │ account_balances_current (ADR 0035: history dropped) │                     │
│  └──────────────────────────┬───────────────────────────┘                     │
│                             │                                                 │
│  API LAYER                  │                                                 │
│  ┌──────────────────────────▼──────────┐  ┌────────────────────────────────┐  │
│  │ API Gateway → Lambda (Rust/axum)    │  │ CloudFront CDN                 │  │
│  │ REST, throttling                    │  │ React SPA + static assets      │  │
│  └─────────────────────────────────────┘  └────────────────────────────────┘  │
└───────────────────────────────────────────────────────────────────────────────┘

Connections:
  Stellar network peers → Galexie (Captive Core, live ledger stream)
  Stellar history archives → `backfill-runner` (production) or `backfill-bench` (benchmark) CLI on developer workstation (one-time, per ADR 0010)
  Galexie → S3 (LedgerCloseMeta XDR files)
  S3 PutObject event → Lambda Ledger Processor
  Lambda Ledger Processor → ClickHouse over mTLS (write) — AWS leaves the VPC here
  Lambda Rust/axum API → ClickHouse over mTLS (read)
  React SPA → API Gateway → Lambda Rust/axum API
```

### 3.2 Deployment Model

The AWS side runs in a **dedicated AWS sub-account owned by Rumble Fish**, in
`eu-central-1` (moved from `us-east-1` by task 0239). The datastore is **not** in AWS —
it is a Hetzner dedicated server, reached over mTLS. There is one environment; see
[`docs/deployment.md`](../deployment.md).

The private-subnet topology this section originally described was replaced by task 0239:
Lambdas were taken **out of the VPC** (removing the NAT Gateway), and Galexie moved to a
**public** subnet with `assignPublicIp` enabled.

```
┌─ AWS eu-central-1 ──────────────────────────────────────────────────────────┐
│                                                                             │
│  CloudFront CDN            API Gateway                                      │
│        │                        │                                           │
│        │                        ▼                                           │
│        │            Lambda (Rust/axum API)          ── no VPC ──┐            │
│        │            Lambda (Ledger Processor)       ── no VPC ──┤            │
│        │                                                        │            │
│  ┌─ VPC (minimal) ──────────────────────────────────────────┐   │            │
│  │  ┌─ Public Subnet ────────────────────────────────────┐  │   │            │
│  │  │  ECS Fargate — Galexie (assignPublicIp ENABLED)    │  │   │            │
│  │  │            │ writes LedgerCloseMeta XDR            │  │   │            │
│  │  │            ▼                                        │  │   │            │
│  │  │  S3  stellar-ledger-data ──PutObject──> SQS doorbell │  │   │            │
│  │  └─────────────────────────────────────────────────────┘  │   │            │
│  └───────────────────────────────────────────────────────────┘   │            │
└──────────────────────────────────────────────────────────────────┼────────────┘
                                                                   │ mTLS
                                                                   ▼
                              ┌──────────────────────────────────────────────┐
                              │ Hetzner  ch-prod-01                          │
                              │  Caddy (client-cert CN allow-list)           │
                              │    └─> ClickHouse (block explorer schema)    │
                              │  mdadm RAID 1 · weekly Borg ─> BX21          │
                              └──────────────────────────────────────────────┘

  Route 53 ──> CloudFront CDN
  Lambda ····> Secrets Manager (mTLS client cert + credentials, via SM Extension)
  Lambda ····> CloudWatch / X-Ray (monitoring — AWS side only; the CH box is
               monitored on the box, not in CloudWatch)
```

### 3.3 Component Ownership

**Hosted by Rumble Fish (AWS sub-account):**

| Component                               | Service                              | Role                                                                                                                                                                                                                                                                                                                                          |
| --------------------------------------- | ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Galexie process                         | ECS Fargate (1 task, continuous)     | Streams live ledger data from Stellar network to S3                                                                                                                                                                                                                                                                                           |
| Historical backfill (`backfill-runner`) | Developer workstation CLI (ADR 0010) | Streams history archives locally; writes directly to ClickHouse. Production tool (task 0145). See [`docs/backfills.md`](../backfills.md).                                                                                                                                                                                                     |
| S3 bucket `stellar-ledger-data`         | AWS S3                               | Receives `LedgerCloseMeta` XDR files; triggers Ledger Processor                                                                                                                                                                                                                                                                               |
| Lambda — Ledger Processor               | AWS Lambda (S3 event-driven)         | Parses XDR; writes explorer records and derived state to ClickHouse over mTLS                                                                                                                                                                                                                                                                 |
| Lambda — Rust/axum API handlers         | AWS Lambda (per API Gateway route)   | Serves all public API requests                                                                                                                                                                                                                                                                                                                |
| ClickHouse                              | Hetzner dedicated `ch-prod-01`       | The block explorer database, per [ADR 0047](../../lore/2-adrs/0047_clickhouse-primary-api-datastore.md). Single node, Ubuntu 24.04 on mdadm RAID 1, reached only over mTLS via Caddy. The RDS PostgreSQL this row used to name was never deployed — see the Store-status note in the intro.                                                   |
| API Gateway                             | AWS API Gateway                      | REST API, throttling, request validation, response caching                                                                                                                                                                                                                                                                                    |
| Edge protection (API hostname only)     | Cloudflare                           | WAF managed rules, unmetered DDoS, Managed Challenge, rate limiting on `api-sorobanscan.rumblefishdev.com`; origin locked to Cloudflare by a secret request header. AWS WAF was dropped — both WebACLs — per [ADR 0048](../../lore/2-adrs/0048_cloudflare-edge-over-aws-waf.md) and task 0302. The CloudFront frontend has no edge filtering. |
| CloudFront CDN                          | AWS CloudFront                       | Serves React frontend                                                                                                                                                                                                                                                                                                                         |
| Swagger UI                              | utoipa-swagger-ui `/api-docs`        | OpenAPI spec + interactive documentation                                                                                                                                                                                                                                                                                                      |
| EventBridge Scheduler                   | AWS EventBridge                      | Cron triggers for operational tasks (e.g. partition management)                                                                                                                                                                                                                                                                               |
| Secrets Manager                         | AWS Secrets Manager                  | DB credentials, non-browser integration keys                                                                                                                                                                                                                                                                                                  |
| CloudWatch + X-Ray                      | AWS CloudWatch                       | Logs, metrics, alarms, distributed tracing                                                                                                                                                                                                                                                                                                    |
| CI/CD pipeline                          | GitHub Actions → AWS CDK             | Infrastructure-as-code deploy                                                                                                                                                                                                                                                                                                                 |

**External services consumed (read-only):**

| External service                             | Purpose                        | Failure impact                                       |
| -------------------------------------------- | ------------------------------ | ---------------------------------------------------- |
| Stellar network peers (Galexie Captive Core) | Live ledger data source        | Critical — mitigated by connecting to multiple peers |
| Stellar public history archives              | Historical backfill (one-time) | Non-critical after backfill completes                |

No external APIs (Horizon, Soroswap, Aquarius, Soroban RPC) are required for core
functionality. All data flows from the canonical ledger.

Public browser traffic is anonymous and read-only. The SPA must not embed API keys or
shared secrets. Abuse control for public traffic is enforced at the ingress layer through
API Gateway throttling/request validation and, on the API hostname, the Cloudflare edge.
There is no AWS WAF — both WebACLs were dropped
([ADR 0048](../../lore/2-adrs/0048_cloudflare-edge-over-aws-waf.md), task 0302) — and the
CloudFront frontend distribution has no edge filtering: it serves a static, edge-cached
bundle from a private S3 origin, so managed injection rules have no application to
protect. If API keys are later enabled, they are reserved for trusted non-browser
consumers and are not required for normal explorer browsing.

### 3.4 Tech Stack

| Component     | Technology                    | Purpose                                                                                  |
| ------------- | ----------------------------- | ---------------------------------------------------------------------------------------- |
| Ingestion     | Galexie (ECS Fargate)         | Streams `LedgerCloseMeta` XDR from Stellar network to S3                                 |
| XDR parsing   | `stellar-xdr` crate (Rust)    | Deserializes all XDR types in Ledger Processor Lambda                                    |
| API Framework | axum / Rust (per ADR 0005)    | Modular REST API with utoipa OpenAPI                                                     |
| Compute       | AWS Lambda (ARM/Graviton2)    | Serverless; auto-scaling                                                                 |
| Gateway       | AWS API Gateway               | Request routing, throttling, validation, response caching                                |
| Edge Security | Cloudflare (API host only)    | Managed rules, IP reputation, DDoS, rate limiting; no AWS WAF, no edge filter on the SPA |
| Database      | ClickHouse (Hetzner, mTLS)    | Block explorer schema; MergeTree family, `intDiv(ledger_sequence, 500000)` partitioning  |
| CDN           | CloudFront                    | Static asset delivery for frontend                                                       |
| DNS           | Route 53                      | Domain management                                                                        |
| Monitoring    | CloudWatch + X-Ray            | Logging, distributed tracing, alarms                                                     |
| Secrets       | Secrets Manager               | Database credentials, non-browser integration keys                                       |
| IaC           | AWS CDK (TypeScript)          | All infrastructure defined as code                                                       |
| CI/CD         | GitHub Actions → `cdk deploy` | Automated deployment on merge to main                                                    |

### 3.5 Environments

| Environment    | Purpose      | Database                           |
| -------------- | ------------ | ---------------------------------- |
| **Production** | Live service | ClickHouse on Hetzner `ch-prod-01` |

**There is exactly one environment.** The multi-environment model this section
originally described (dev / staging / production, each with its own RDS) was never
built. AWS-side staging was destroyed by task 0249 on 2026-05-21 and never gained a
Hetzner counterpart; every deploy is run manually from an operator laptop. See
[`docs/deployment.md`](../deployment.md) before trusting any `staging` command still
lying around the repo.

Production durability and security baselines: **weekly** Borg backup (Sunday 03:30 UTC,
`/etc/cron.d/ch-backup`) pushing a FREEZE-consistent tree to a Hetzner BX21 Storage Box,
retention `--keep-weekly 4`; mdadm RAID 1 on the box itself; mTLS on every AWS →
ClickHouse connection, terminated by Caddy and gated on client-certificate CN; SSE-S3
(AES256) encryption for the public-XDR `stellar-ledger-data` bucket (KMS avoided there
to drop per-object request cost — task 0278); and TLS on public ingress.

> Note the RPO this implies: a weekly backup means up to seven days of ledgers must be
> re-ingested with `backfill-runner` after a restore — the Lambdas do **not** re-deliver
> the rolled-back range. See [`docs/backups.md`](../backups.md).

### 3.6 Scalability

| Component            | Mechanism                                    | Trigger                                                   |
| -------------------- | -------------------------------------------- | --------------------------------------------------------- |
| **API**              | Lambda auto-scale (up to 50 concurrent)      | On-demand                                                 |
| **Ledger Processor** | Lambda auto-scale (S3 event-driven)          | Per ledger file                                           |
| **ClickHouse**       | Vertical — the box is a dedicated server     | Default                                                   |
|                      | Materialized views for aggregated statistics | Default                                                   |
|                      | Add a second CH node (replicated MergeTree)  | Deferred per ADR 0047                                     |
| **CDN**              | CloudFront scales automatically              | N/A                                                       |
| **Redundancy**       | mdadm RAID 1 + weekly Borg to BX21           | Already in place; no HA pair — a box loss costs a restore |

### 3.7 Monitoring and Alerting

| Alarm                           | Threshold (production)                                  | Action      |
| ------------------------------- | ------------------------------------------------------- | ----------- |
| Galexie ingestion lag           | 0 new S3 ledger files (ingest-queue doorbells) in 5 min | SNS → Slack |
| Galexie ephemeral storage       | >60% of task ephemeral disk, sustained 3×5 min          | SNS → Slack |
| Ledger Processor error rate     | >1% of Lambda invocations in error                      | SNS → Slack |
| Indexer CH write failures       | >10 post-retry hard-failure log lines in 5 min          | SNS → Slack |
| Ledger Processor DLQ depth      | >0 messages                                             | SNS → Slack |
| Enrichment DLQ depth            | >0 messages                                             | SNS → Slack |
| Enrichment worker error rate    | >1% of Lambda invocations in error                      | SNS → Slack |
| API Gateway 5xx rate            | >0.5% of requests                                       | SNS → Slack |
| Origin-lock canary (flag-gated) | direct-origin bypass answers instead of 403             | SNS → Slack |

The thresholds above are the production baseline — and, since production is the only
environment, the only baseline. The authoritative definitions (including each
alarm's `treatMissingData` rationale) live in
`infra/src/lib/stacks/cloudwatch-stack.ts`. ClickHouse host alarms (CPU, disk)
were removed from CloudWatch in task 0239 — the database box is monitored on the
box itself, outside AWS (see `infra-hetzner/`).

CloudWatch dashboards expose: Galexie S3 file freshness, Ledger Processor duration and
error rate, API latency (p50/p95/p99), and highest indexed ledger sequence vs. network
tip. ClickHouse host metrics are **not** in CloudWatch — the box is outside AWS.

---

## 4. Indexing Pipeline (Galexie)

### 4.1 Overview

Indexing uses **self-hosted Galexie** running on ECS Fargate. Galexie connects to Stellar
network peers via Captive Core, exports one `LedgerCloseMeta` XDR file per ledger close
to S3, and a Lambda function processes each file as it arrives.

```
Stellar Network (mainnet peers)
        │
        ▼ (Captive Core / ledger stream)
┌──────────────────────────────────┐
│  Galexie — ECS Fargate (1 task)  │
│  Continuously running            │
│  Exports one file per ledger     │
│  (~1 file every 5–6 seconds)     │
└──────────────┬───────────────────┘
               │ LedgerCloseMeta XDR (zstd-compressed)
               ▼
┌──────────────────────────────────┐
│  S3: stellar-ledger-data/        │
│  ledgers/{seq_start}-{seq_end}   │
│                    .xdr.zst      │
└──────────────┬───────────────────┘
               │ S3 ObjectCreated → SNS {env}-ledger-events
               │ → SQS ledger-ingest (rawMessageDelivery; fan-out, task 0306)
               ▼
┌─────────────────────────────────────────────────────────┐
│  Lambda "Ledger Processor"  (event-driven, per file)    │
│  1. Download + decompress XDR                           │
│  2. Parse LedgerCloseMeta via Rust `stellar-xdr` crate  │
│  3. Extract ledger header → ledgers row                 │
│  4. Extract transactions: hash (BYTEA32), source_id,    │
│     fee, successful, application_order, operation_count,│
│     has_soroban, inner_tx_hash → transactions           │
│     (no raw envelope/result XDR — ADR 0029)             │
│  5. Aggregate operations by identity (type, source_id,  │
│     destination_id, contract_id, asset_code,            │
│     asset_issuer_id, pool_ids) →`operations_appearances`│
│     with `amount BIGINT` counting collapsed duplicates  │
│     (ADR 0163 — no transfer_amount, no application_order,│
│     no details JSONB)                                   │
│  6. Resolve StrKeys → surrogate ids for accounts and    │
│     soroban_contracts (ADRs 0026/0030)                  │
│  7. Soroban events: one appearance row per              │
│     (contract, tx, ledger) trio → soroban_events_…      │
│     (full event detail fetched at read time — ADR 0033) │
│  8. Soroban invocations: appearance rows + root         │
│     caller_id → soroban_invocations_appearances         │
│     (per-node detail fetched at read time — ADR 0034)   │
│  9. Contract deployments + classic account state →      │
│     soroban_contracts, wasm_interface_metadata,         │
│     accounts, account_balances_current                  │
│ 10. Detect SEP-41 token contracts, NFT contracts,       │
│     classic LPs → assets, nfts, liquidity_pools,        │
│     nft_ownership, lp_positions                         │
│ 11. Flush the `ledgers` row LAST, after every other      │
│     insert has ack'd — it is the commit marker           │
└─────────────────────────────────────────────────────────┘
               │
               ▼
       ClickHouse — Hetzner ch-prod-01 (block explorer schema — Section 6)
```

### 4.2 What `LedgerCloseMeta` Contains

The `LedgerCloseMeta` XDR produced by Galexie contains the complete ledger close.
Everything the block explorer needs to populate its typed summary columns is
present; no private chain API is required at ingestion time. The public Stellar
ledger archive is a read-time dependency for heavy-field endpoints
([ADR 0029](../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)),
not an ingest-time one.

| Data needed                                       | Where it lives in LedgerCloseMeta                                         |
| ------------------------------------------------- | ------------------------------------------------------------------------- |
| Ledger sequence, close time, protocol version     | `LedgerHeader`                                                            |
| Transaction hash, source account, fee, success    | `TransactionEnvelope` + `TransactionResult`                               |
| Operation type and details                        | `OperationMeta` per transaction                                           |
| Soroban invocation (function, args, return value) | `InvokeHostFunctionOp` in envelope + `SorobanTransactionMeta.returnValue` |
| CAP-67 contract events (type, topics, data)       | `SorobanTransactionMeta.events`                                           |
| Contract deployment (C-address, WASM hash)        | `LedgerEntryChanges` (CONTRACT type)                                      |
| Account balance changes                           | `LedgerEntryChanges` (ACCOUNT type)                                       |
| Liquidity pool state                              | `LedgerEntryChanges` (LIQUIDITY_POOL type)                                |

### 4.3 Historical Backfill

Per [ADR 0010](../../lore/2-adrs/0010_local-backfill-over-fargate.md), historical
backfill is not a production Fargate task. It runs as a **local CLI tool**
(`crates/backfill-runner` for production, `crates/backfill-bench` for benchmarking)
on a developer workstation that streams from Stellar's
**public history archives** (the same archives Horizon used for `db reingest`),
invokes the same `process_ledger` pipeline used by the Lambda, and writes
directly to ClickHouse. Production is the only target — see
[`docs/backfills.md`](../backfills.md), which also records the non-obvious
constraint that `repair-tier1` is **mandatory** after a parallel or `--reindex`
run.

- **Scope:** from Soroban mainnet activation ledger (late 2023) to the present
- **Parallelism:** backfill runs in configurable ledger-range batches. Batches
  may execute in parallel only when they own non-overlapping ledger ranges and
  preserve deterministic replay semantics
- **Timing:** one-time batch during Phase 1 (Deliverable 1); live ingestion
  continues in parallel via the Galexie → S3 → Ledger Processor path and
  live-derived state remains authoritative for the newest ledgers
- **No production infrastructure:** no Fargate task, no ECS task definitions,
  no EventBridge schedule; the CLI runs on demand from an operator's machine

### 4.4 Background Workers

| Worker               | Trigger                     | Role                                                                |
| -------------------- | --------------------------- | ------------------------------------------------------------------- |
| **Ledger Processor** | S3 PutObject (~every 5–6 s) | Primary ingestion — parses XDR, writes all chain data to ClickHouse |

### 4.5 Operational Characteristics

**Normal operation (live):**

```
Galexie (ECS Fargate) → S3 (~5-6 s per ledger)
                      → Lambda Ledger Processor (~<10 s from ledger close to DB write)
```

**Recovery from Galexie restart:** Galexie is checkpoint-aware. On restart it reads the
last exported ledger sequence and resumes from there. No manual intervention required.

**Recovery from Ledger Processor failure:** once a doorbell is in the
`ledger-ingest` queue, a failing invocation is redelivered automatically (per
`maxReceiveCount`, then `ledger-processor-dlq`, recoverable via SQS
redrive-to-source). Note this covers only the SQS → Lambda cycle; the upstream
S3 → SNS → SQS hops have no DLQ, so a doorbell dropped before it reaches the
queue is _not_ recoverable via `maxReceiveCount`. The durable backstop for any
lost doorbell is that the file remains in S3 (retained indefinitely, ADR 0006)
and the next doorbell's reconcile reads `max(sequence)` from CH and replays the
contiguous gap forward — a single missed doorbell self-heals on the next ledger;
a sustained SNS outage requires a manual re-trigger by S3 key.

**Replay artifact retention:** the `stellar-ledger-data` bucket retains files indefinitely
(ADR 0006). No automatic deletion. This supports replay and post-incident validation at any
point. Lifecycle rules can be added later if storage costs grow. Previously planned as 30 days production / 7
days. Lifecycle expiration happens only after that minimum replay window.

**Idempotency and ordering:** ledger sequence is the canonical ordering key. Processing is
replay-safe: immutable ledger-scoped writes happen transactionally per ledger, and
reprocessing the same ledger replaces or de-duplicates that ledger's immutable rows rather
than creating duplicates. Derived-state upserts (`accounts`, `assets`, `nfts`,
`liquidity_pools`) apply only when the incoming ledger sequence is newer than or equal to
the stored watermark (`last_seen_ledger` / `last_updated_ledger`), so an older backfill
batch cannot overwrite fresher live state.

**Schema migrations:** there is no migration mechanism. The single
`crates/db-clickhouse/schema/init.sql` is applied idempotently
(`CREATE ... IF NOT EXISTS`) and IS the schema of record — the crate's README
says so in as many words. Evolving a live table is a hand-run `ALTER` on the
box, and because the ClickHouse driver validates inserts client-side, a
struct/table mismatch fails writers at runtime rather than at build time, so
the ALTER and the redeploy are one manual window. (Corrected 2026-08-19: this
paragraph previously claimed versioned migrations managed via CDK and run in
CI/CD, which was never true. A numbered migration ladder is proposed work,
not current state.)

**Protocol upgrades:** when Stellar introduces a new CAP that changes `LedgerCloseMeta`
structure, we update the pinned `stellar-xdr` Rust crate version; the frontend consumes
typed API responses via OpenAPI-generated TS client (task 0096). Protocol
upgrades are infrequent and well-announced in advance.

**Open-source re-deployability:** the full CDK stack is public; Stellar or any third party
can fork the repository and deploy the entire system in a fresh AWS account.

---

## 5. XDR Parsing

### 5.1 Parsing Strategy

XDR parsing happens in two places, each with a different scope:

- **Ledger Processor Lambda (ingestion time):** the primary parsing path. Every
  ledger's `LedgerCloseMeta` is fully deserialized using the Rust `stellar-xdr`
  crate (ADR 0004). The ingestion path extracts the **typed summary columns**
  that populate `transactions`, `operations_appearances`, `soroban_contracts`, `assets`,
  `nfts`, `liquidity_pools`, and the surrogate-keyed hubs (`accounts`,
  `soroban_contracts`), plus the appearance-index rows for
  `soroban_events_appearances` and `soroban_invocations_appearances`. No raw XDR
  is written to ClickHouse
  ([ADR 0029](../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)).

- **Backend API (request time):** the envelope / result / result-meta payloads
  and the parsed event / invocation tree for E3 and E14 are fetched from the
  **public Stellar ledger archive** (`.xdr.zst` files), decompressed, parsed
  with the same `stellar-xdr` crate (shared via `crates/xdr-parser`), and
  merged into the response on the fly.

### 5.2 Data Extracted at Ingestion (Ledger Processor)

**From `LedgerHeader`:**

- `sequence`, `closeTime`, `protocolVersion`, `baseFee`

**From `TransactionEnvelope` + `TransactionResult`:**

- `hash` (32-byte, stored as `BYTEA` per
  [ADR 0024](../../lore/2-adrs/0024_hashes-bytea-binary-storage.md))
- `source_id` resolved to `accounts.id` at ingest
  ([ADR 0026](../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md))
- `fee_charged`, `successful`, `application_order`, `operation_count`,
  `has_soroban`, `inner_tx_hash`
- **No raw envelope / result / result-meta XDR is stored.** Those are fetched
  on demand from the public archive when E3 needs them.

**From `OperationMeta` per transaction:**

- Operation `type` (`SMALLINT` Rust `OperationType` enum per
  [ADR 0031](../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md))
- Typed summary columns: `source_id`, `destination_id`, `contract_id` (surrogate
  FK per [ADR 0030](../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md)),
  `asset_code`, `asset_issuer_id`, `pool_id`, `transfer_amount`
- No per-operation `details` JSONB — list endpoints use the typed columns; the
  detail endpoint re-derives from the archive

**From `SorobanTransactionMeta.events`:**

- An **appearance row** per (contract, transaction, ledger) tuple in
  `soroban_events_appearances` with an `amount` count of non-diagnostic events
  (ADR 0033). Full event detail (type, topics, data, per-event index) is not
  persisted; E14 re-expands it from the archive via
  `xdr_parser::extract_events`
- Known SEP-41 / NFT transfer patterns also drive derived-state upserts on
  `assets`, `nfts`, and `nft_ownership`. Per-account Soroban token holdings
  are explicitly out of scope: `account_balances_current` (§4.17 of the
  schema overview) carries only classic balances (native XLM + trustlines)
  per ADR 0035; Soroban `ContractData` `Balance(address)` entries are not
  extracted into per-account state. See archived task 0138 (closed
  2026-05-05 as scope-out per current technical design) and the §4.2
  entry-type map above, which routes "Account balance changes" to
  `LedgerEntryChanges (ACCOUNT type)` only — `CONTRACT_DATA` entries are
  the source of contract events, deployments, and per-asset registry
  upserts (`assets`, `nfts`), not per-account holdings.

**From `SorobanTransactionMeta.diagnosticEvents` / invocation tree:**

- An appearance row per (contract, transaction, ledger) in
  `soroban_invocations_appearances` with `amount` (node count) and the
  root-level `caller_id` (ADR 0034). Per-node detail (function name,
  arguments, return value, depth) is re-expanded at read time by
  `xdr_parser::extract_invocations`

**From `LedgerEntryChanges`:**

- Contract deployments: `soroban_contracts` row (contract_id, wasm_hash BYTEA,
  deployer_id surrogate, deployed_at_ledger, is_sac)
- Classic account state: `accounts` row (account_id, first/last-seen ledgers,
  sequence number, home_domain); balances to `account_balances_current`
- Liquidity pool state: `liquidity_pools` row with typed `asset_*_type`
  SMALLINT + `asset_*_code` + `asset_*_issuer_id` per leg, plus a
  `liquidity_pool_snapshots` time-series row

### 5.3 Soroban-Specific Handling

- **CAP-67 events** are indexed at ingest (one row per contract × transaction
  tuple in `soroban_events_appearances`, with a non-diagnostic-event count).
  Full decoded event detail is served at read time from the public archive via
  E14 — there is no `soroban_events` JSONB table
- **Return values** — the `invokeHostFunction` return value and the per-node
  invocation tree live at read time in the archive and are expanded on demand
  by `xdr_parser::extract_invocations`; the DB only indexes the appearance per
  trio in `soroban_invocations_appearances`
- **Invocation tree rendering** — the transaction-detail page fetches
  `envelope.xdr.zst` + `meta.xdr.zst` from the archive and renders the full
  per-node tree on demand (no `transactions.operation_tree` JSONB)
- **Contract interface** — function signatures (names, parameter types) are
  extracted from the contract WASM on upload and stored as JSONB in
  `wasm_interface_metadata.metadata`, keyed by `wasm_hash` (BYTEA 32). The
  contract page joins `soroban_contracts.wasm_hash → wasm_interface_metadata`
  for display ([ADR 0022](../../lore/2-adrs/0022_schema-correction-and-token-metadata-enrichment.md))

### 5.4 Error Handling

- **Malformed XDR at ingest** — if the Rust parser returns an error on a
  transaction, the indexer logs the hash, writes the typed summary columns it
  was able to extract, and sets `transactions.parse_error = true`. The
  transaction is still displayed with the partial columns. The detail page
  retries the archive fetch; if that also fails, the UI shows a decode-failure
  indicator
- **Archive fetch failure at read time** — E3 / E14 return an upstream-error
  envelope with retry-after semantics; list endpoints are unaffected since they
  do not call the archive
- **Unknown operation types** — new protocol versions may introduce operation types not
  yet supported by the SDK. These are rendered as "Unknown operation" with raw XDR shown,
  and a CloudWatch alarm is raised to trigger an SDK update.

---

## 6. Database Schema

The block explorer owns its full schema. All chain data is stored in ClickHouse;
there is no dependency on an external database.

This section is the narrative overview. Authoritative DDL for every table (column types,
constraints, indexes, partition names) lives in
[`database-schema/database-schema-overview.md`](database-schema/database-schema-overview.md),
kept in sync with the ClickHouse schema `crates/db-clickhouse/schema/init.sql` per
[ADR 0032](../../lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md).

> The DDL and key types in this section are shown in their historical PostgreSQL
> notation for readability (Postgres was retired in task 0244). For the live ClickHouse
> form — `ReplacingMergeTree`, `Int64` surrogates, `Decimal128(7)`,
> `intDiv(ledger_sequence, 500000)` partitioning — see `database-schema-overview.md`
> and `init.sql`.

Cross-cutting schema disciplines applied to every table:

- **Surrogate primary keys** on `accounts` and `soroban_contracts`
  ([ADR 0026](../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md),
  [ADR 0030](../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md)):
  `BIGSERIAL id` is the join key; the `VARCHAR(56)` StrKey (`account_id`, `contract_id`)
  stays as a UNIQUE natural key for display, route lookup, and E22 search.
  Every FK column elsewhere in the schema targets the surrogate `id`.
- **Binary hashes** ([ADR 0024](../../lore/2-adrs/0024_hashes-bytea-binary-storage.md)):
  every 32-byte chain hash (`ledgers.hash`, `transactions.hash`, `wasm_hash`, `pool_id`)
  is `BYTEA` with `CHECK (octet_length(...) = 32)` — rendered as lowercase hex at the
  API layer.
- **SMALLINT enums** ([ADR 0031](../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md)):
  every closed-domain "type" column (`operations_appearances.type`, `assets.asset_type`,
  `soroban_contracts.contract_type`, `nft_ownership.event_type`, etc.) is `SMALLINT`
  backed by a Rust `#[repr(i16)]` enum with a `CHECK` range constraint and a
  `<name>_name(ty)` SQL helper for psql/BI.
- **Range partitioning on ledger sequence** for high-volume child tables
  (see §6.12). On ClickHouse each MergeTree table declares
  `PARTITION BY intDiv(sequence, 500000)` (500k-ledger blocks) and the engine
  creates the parts automatically on insert — there is no provisioning step.
  (The PG-era partition-management Lambda `crates/db-partition-mgmt`, which
  pre-created monthly `<table>_y{YYYY}m{MM}` partitions, was removed with the
  PG→CH cutover — task 0241.)
- **No raw XDR in the DB** ([ADR 0029](../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)):
  `transactions` carries only typed summary columns. Full envelope / result / result-meta
  XDR for E3 `/transactions/:hash` and decoded event / invocation payloads for
  E14 `/contracts/:id/events` are fetched at request time from the public Stellar ledger
  archive (`.xdr.zst` files) and re-parsed on demand.

The illustrative DDL below shows the main structural features of each table; for the
full CHECKs, indexes, and FK wiring see the overview doc. Snippets use `…` where
additional fields exist in the live schema.

### 6.1 Ledgers

```sql
CREATE TABLE ledgers (
    sequence          BIGINT      PRIMARY KEY,
    hash              BYTEA       NOT NULL UNIQUE,             -- 32-byte (ADR 0024)
    closed_at         TIMESTAMPTZ NOT NULL,
    protocol_version  INTEGER     NOT NULL,
    transaction_count INTEGER     NOT NULL,
    base_fee          BIGINT      NOT NULL
    -- CHECK (octet_length(hash) = 32)
);
```

### 6.2 Transactions

```sql
CREATE TABLE transactions (
    id                BIGSERIAL   NOT NULL,
    hash              BYTEA       NOT NULL,                    -- 32-byte (ADR 0024)
    ledger_sequence   BIGINT      NOT NULL,
    application_order SMALLINT    NOT NULL,
    source_id         BIGINT               REFERENCES accounts(id),  -- ADR 0026 surrogate; NULLable for Variant A parse_error tx (lore-0209)
    fee_charged       BIGINT      NOT NULL,
    inner_tx_hash     BYTEA,                                   -- fee-bump inner, 32-byte
    successful        BOOLEAN     NOT NULL,
    operation_count   SMALLINT    NOT NULL,
    has_soroban       BOOLEAN     NOT NULL DEFAULT false,
    parse_error       BOOLEAN     NOT NULL DEFAULT false,
    created_at        TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at)                               -- partition-key rule
) PARTITION BY RANGE (created_at);
-- hash uniqueness lives in the companion `transaction_hash_index` table,
-- because a partitioned parent cannot carry a hash-only UNIQUE (the partition
-- key would have to be part of every unique index).
```

No raw envelope / result / result-meta XDR and no decoded `operation_tree` JSONB are
stored on the row — those fields are fetched and parsed on demand per ADR 0029.

### 6.3 Operations — Appearance Index

Per task 0163, `operations` was collapsed into an appearance index (pattern from
ADRs 0033/0034) and renamed to `operations_appearances`. One row per distinct
operation identity per transaction, `amount BIGINT` counts collapsed duplicates;
per-op detail re-materialised from XDR at read time.

```sql
CREATE TABLE operations_appearances (
    id                BIGSERIAL    NOT NULL,
    transaction_id    BIGINT       NOT NULL,
    type              SMALLINT     NOT NULL,                               -- ADR 0031
    source_id         BIGINT       REFERENCES accounts(id),                -- ADR 0026
    destination_id    BIGINT       REFERENCES accounts(id),                -- ADR 0026
    contract_id       BIGINT       REFERENCES soroban_contracts(id),       -- ADR 0030
    asset_code        VARCHAR(12),
    asset_issuer_id   BIGINT       REFERENCES accounts(id),                -- ADR 0026
    pool_id           BYTEA,                                               -- 32-byte LP hash
    amount            BIGINT       NOT NULL,                               -- collapsed-duplicate count
    ledger_sequence   BIGINT       NOT NULL,
    created_at        TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE,
    CONSTRAINT uq_ops_app_identity UNIQUE NULLS NOT DISTINCT
        (transaction_id, type, source_id, destination_id,
         contract_id, asset_code, asset_issuer_id, pool_id,
         ledger_sequence, created_at)
) PARTITION BY RANGE (created_at);
```

`transfer_amount` and `application_order` are no longer stored; per-operation
JSONB `details` was never stored. The typed summary columns above support
filtered list endpoints; full decoded payloads come from the archive via
`runtime_enrichment::stellar_archive` extractors at request time (ADR 0029).

### 6.4 Soroban Contracts

```sql
CREATE TABLE soroban_contracts (
    id                      BIGSERIAL   PRIMARY KEY,                             -- ADR 0030 surrogate
    contract_id             VARCHAR(56) NOT NULL UNIQUE,                         -- StrKey natural key
    wasm_hash               BYTEA       REFERENCES wasm_interface_metadata(wasm_hash),  -- 32-byte (ADR 0024)
    wasm_uploaded_at_ledger BIGINT,
    deployer_id             BIGINT      REFERENCES accounts(id),                 -- ADR 0026
    deployed_at_ledger      BIGINT,
    contract_type           SMALLINT,                                            -- ADR 0031, nullable
    is_sac                  BOOLEAN     NOT NULL DEFAULT false,
    name                    VARCHAR(256),                                        -- ADR 0042, on-chain Symbol("name")
    search_vector           TSVECTOR GENERATED ALWAYS AS (
                                to_tsvector('simple', COALESCE(name, '') || ' ' || contract_id)
                            ) STORED
);
```

### 6.5 Soroban Invocations — Appearance Index

Per [ADR 0034](../../lore/2-adrs/0034_soroban-invocations-appearances-read-time-detail.md),
the former per-node `soroban_invocations` table was replaced with a pure appearance
index; full per-node decode happens at read time from the public archive
(pattern shared with ADR 0033 for events).

```sql
CREATE TABLE soroban_invocations_appearances (
    contract_id      BIGINT       NOT NULL REFERENCES soroban_contracts(id),  -- ADR 0030
    transaction_id   BIGINT       NOT NULL,
    ledger_sequence  BIGINT       NOT NULL,
    caller_id        BIGINT       REFERENCES accounts(id),                    -- ADR 0026
    amount           INTEGER      NOT NULL,                                   -- invocation nodes in trio
    created_at       TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (contract_id, transaction_id, ledger_sequence, created_at)
) PARTITION BY RANGE (created_at);
```

### 6.6 Soroban Events — Appearance Index

Mirrors §6.5 for contract events (ADR 0033):

```sql
CREATE TABLE soroban_events_appearances (
    contract_id     BIGINT       NOT NULL REFERENCES soroban_contracts(id),   -- ADR 0030
    transaction_id  BIGINT       NOT NULL,
    ledger_sequence BIGINT       NOT NULL,
    amount          BIGINT       NOT NULL,                                    -- non-diagnostic events in trio
    created_at      TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (contract_id, transaction_id, ledger_sequence, created_at)
) PARTITION BY RANGE (created_at);
```

Parsed event type, topics, and data live at read time in the public archive and are
re-expanded on demand via `xdr_parser::extract_events`.

### 6.7 Assets

Renamed from `tokens` in
[ADR 0036](../../lore/2-adrs/0036_rename-tokens-to-assets.md) / task 0154. Four asset
classes under a single registry (`native`, `classic_credit`, `sac`, `soroban`).

```sql
CREATE TABLE assets (
    id           SERIAL        PRIMARY KEY,
    asset_type   SMALLINT      NOT NULL,  -- TokenAssetType: 0=native, 1=classic_credit, 2=sac, 3=soroban (ADR 0031)
    asset_code   VARCHAR(12),
    issuer_id    BIGINT        REFERENCES accounts(id),              -- ADR 0026
    contract_id  BIGINT        REFERENCES soroban_contracts(id),     -- ADR 0030
    name         VARCHAR(256),
    total_supply NUMERIC(28,7),                                      -- populated by metadata worker (ADR 0022)
    holder_count INTEGER,
    description  TEXT,                                               -- typed SEP-1 (ADR 0023)
    icon_url     VARCHAR(1024),                                      -- ditto
    home_page    VARCHAR(256)                                        -- ditto
    -- CHECK ck_assets_identity: required NULL/NOT NULL columns per asset_type
    -- partial UNIQUE indexes enforce one row per logical asset
);
```

### 6.8 Accounts

```sql
CREATE TABLE accounts (
    id                BIGSERIAL    PRIMARY KEY,               -- ADR 0026 surrogate
    account_id        VARCHAR(56)  NOT NULL UNIQUE,           -- StrKey G... natural key
    first_seen_ledger BIGINT       NOT NULL,
    last_seen_ledger  BIGINT       NOT NULL,
    sequence_number   BIGINT       NOT NULL,
    home_domain       VARCHAR(256)
);
```

Balances are not persisted on this row; see `account_balances_current` in
[`database-schema-overview.md` §4.17](database-schema/database-schema-overview.md#417-account-balances-current).
The parallel `account_balance_history` table was dropped per
[ADR 0035](../../lore/2-adrs/0035_drop-account-balance-history.md).

### 6.9 NFTs

```sql
CREATE TABLE nfts (
    id                   SERIAL       PRIMARY KEY,
    contract_id          BIGINT       NOT NULL REFERENCES soroban_contracts(id),   -- ADR 0030
    token_id             VARCHAR(256) NOT NULL,
    collection_name      VARCHAR(256),
    name                 VARCHAR(256),
    media_url            TEXT,
    metadata             JSONB,
    minted_at_ledger     BIGINT,
    current_owner_id     BIGINT       REFERENCES accounts(id),                     -- ADR 0026
    current_owner_ledger BIGINT,
    UNIQUE (contract_id, token_id)
);
-- companion table nft_ownership (partitioned) records mint/transfer/burn history
-- with event_type SMALLINT (NftEventType) per ADR 0031.
```

### 6.10 Liquidity Pools

```sql
CREATE TABLE liquidity_pools (
    pool_id            BYTEA       PRIMARY KEY,                    -- 32-byte pool hash (ADR 0024)
    asset_a_type       SMALLINT    NOT NULL,                       -- XDR AssetType (ADR 0031)
    asset_a_code       VARCHAR(12),
    asset_a_issuer_id  BIGINT      REFERENCES accounts(id),        -- ADR 0026
    asset_b_type       SMALLINT    NOT NULL,
    asset_b_code       VARCHAR(12),
    asset_b_issuer_id  BIGINT      REFERENCES accounts(id),        -- ADR 0026
    fee_bps            INTEGER     NOT NULL,
    created_at_ledger  BIGINT      NOT NULL
);
```

The asset pair is modeled with typed columns per leg (type + code + issuer_id) rather
than JSONB blobs. Current reserves / total_shares are read from the most recent
`liquidity_pool_snapshots` row; per-account LP positions live in `lp_positions`.

### 6.11 Liquidity Pool Snapshots

```sql
CREATE TABLE liquidity_pool_snapshots (
    id              BIGSERIAL     NOT NULL,
    pool_id         BYTEA         NOT NULL REFERENCES liquidity_pools(pool_id),  -- ADR 0024
    ledger_sequence BIGINT        NOT NULL,
    reserve_a       NUMERIC(28,7) NOT NULL,
    reserve_b       NUMERIC(28,7) NOT NULL,
    total_shares    NUMERIC(28,7) NOT NULL,
    tvl             NUMERIC(28,7),
    volume          NUMERIC(28,7),
    fee_revenue     NUMERIC(28,7),
    created_at      TIMESTAMPTZ   NOT NULL,
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);
```

**ClickHouse — USD denomination at read time ([ADR 0053](../../lore/2-adrs/0053_fast-change-offchain-compute-at-read.md)).**
On the ClickHouse primary store ([ADR 0047](../../lore/2-adrs/0047_clickhouse-primary-api-datastore.md)),
`tvl` / `volume` / `fee_revenue` are **not** materialized into these rows by a
write-back worker — the snapshot columns stay unwritten. Fast-change off-chain
values (USD denominations) are computed at **read time** (task 0199, 2026-08)
by joining the prices service's in-cluster views (`prices.price_usd_series` /
`_1h`, `prices.current_price_usd`; natural-identity key, contract pinned in the
prices repo `views.sql`) against the snapshot's on-chain quantities, mapping
`ledger → closed_at → grain-floored price bucket`. This keeps
`liquidity_pool_snapshots` single-writer (indexer only) and avoids the
`ReplacingMergeTree` per-row read-modify-write race. All three series ship:
`tvl = reserve_a·price_a + reserve_b·price_b` (both legs required),
`volume = Σ gross_volume_a·price_a` (per-ledger pricing; `gross_volume_a` from
PathPayment claim atoms, live since 0261 and backfilled by 0266),
`fee_revenue = volume · fee_bps / 10000`. The chart reads the series views at
the interval's grain; the detail endpoint prices latest reserves at each leg's
last hourly close (`price_usd_series_1h`, 48h lookback — not
`current_price_usd`, whose 0-sentinel covers native XLM as of 2026-08) and
sums the last 24h of volume. The pools **list** prices its page the same way,
from one batched close lookup per page (task 0199 Phase A2); `filter[min_tvl]`
is rejected with 400 because a compute-at-read value cannot filter page
membership. No grant is needed to read `prices.*`: the API's `api_reader` CH
user carries no `<grants>` block in `users.d/services.xml` — unlike
`prices_writer`/`prices_reader`, where grants NARROW access — so its
`read_only` profile already covers the database (verified on the box
2026-08-04).

### 6.12 Partitioning and Retention

Partitioned (`PARTITION BY RANGE (created_at)`, monthly):
`transactions`, `operations_appearances`, `transaction_participants`,
`soroban_events_appearances`, `soroban_invocations_appearances`,
`liquidity_pool_snapshots`, `nft_ownership`.

Unpartitioned anchors and registries:
`ledgers`, `transaction_hash_index`, `accounts`, `soroban_contracts`,
`wasm_interface_metadata`, `assets`, `nfts`, `liquidity_pools`, `lp_positions`,
`account_balances_current`.

On ClickHouse, partitions are 500k-ledger blocks (`PARTITION BY
intDiv(sequence, 500000)`) created automatically by the engine on insert —
there is no ahead-of-edge provisioning Lambda (the PG-era
`crates/db-partition-mgmt` was removed with the PG→CH cutover, task 0241).
Ledger and transaction history are kept indefinitely.

---

## 7. Estimates

### 7.1 Effort Breakdown by Project Part

#### A. Design — 35–40 days (runs before / in parallel with Phase 1)

#### B. AWS Architecture + Galexie Infrastructure

| Task                                                              | Days   |
| ----------------------------------------------------------------- | ------ |
| VPC, subnets, security groups, IAM roles (CDK)                    | 4      |
| ECS Fargate cluster + Galexie task definition + S3 bucket         | 5      |
| Galexie configuration and testnet validation                      | 3      |
| Lambda + API Gateway setup (Rust deployment via cargo-lambda-cdk) | 4      |
| CloudFront CDN + Route 53 + TLS                                   | 1      |
| Secrets Manager, CloudWatch dashboards, X-Ray                     | 2      |
| Historical backfill ECS task + monitoring                         | 5      |
| CI/CD pipeline (GitHub Actions → CDK)                             | 4      |
| Staging + production environment parity                           | 4      |
| **Subtotal**                                                      | **32** |

#### C. Data Ingestion Pipeline

| Task                                                                                      | Days   |
| ----------------------------------------------------------------------------------------- | ------ |
| Ledger Processor Lambda — XDR parse + DB write (ledgers, txs, ops, accounts, NFTs, pools) | 6      |
| Ledger Processor — Soroban invocations + CAP-67 events extraction                         | 5      |
| Ledger Processor — contract deployments + asset/NFT/pool detection                        | 4      |
| Backfill validation — gap detection, idempotency checks                                   | 3      |
| Ingestion lag monitoring + alerting                                                       | 2      |
| **Subtotal**                                                                              | **25** |

#### D. Core API Endpoints (axum)

| Task                                                             | Days   |
| ---------------------------------------------------------------- | ------ |
| Rust API scaffolding (axum + utoipa), sqlx setup                 | 3      |
| Network stats endpoint                                           | 1      |
| Transactions endpoints (list + detail + operation tree)          | 9      |
| Ledgers endpoints (list + detail)                                | 3      |
| Accounts endpoints (detail + transactions/history)               | 4      |
| Assets endpoints (list + detail + transactions)                  | 5      |
| Contracts endpoints (detail + interface + invocations + events)  | 9      |
| NFTs endpoints (list + detail + transfers)                       | 5      |
| Liquidity Pools endpoints (list + detail + transactions + chart) | 6      |
| Search endpoint (full-text + prefix matching)                    | 4      |
| XDR decoding service (raw XDR → structured for advanced view)    | 4      |
| Cursor-based pagination                                          | 3      |
| Rate limiting, API key auth, error handling, health checks       | 3      |
| Caching layer (in-memory + CloudFront TTL configuration)         | 3      |
| **Subtotal**                                                     | **62** |

#### E. Frontend Components + API Integration

| Task                                                                 | Days   |
| -------------------------------------------------------------------- | ------ |
| React project scaffolding, routing, design system setup              | 3      |
| Shared components (header, nav, search bar, copy button, timestamps) | 3      |
| Home page (chain overview, latest transactions + ledgers)            | 2      |
| Transactions page (paginated table, filters)                         | 2      |
| Transaction detail page — normal mode (graph/tree view)              | 5      |
| Transaction detail page — advanced mode (raw data, XDR)              | 4      |
| Ledgers page (paginated table)                                       | 1      |
| Ledger detail page                                                   | 2      |
| Account detail page (summary + balances + history)                   | 3      |
| Assets page (list, filters)                                          | 2      |
| Asset detail page (summary + transactions)                           | 2      |
| Contract detail page (summary + interface + invocations + events)    | 7      |
| NFTs page (list, filters)                                            | 2      |
| NFT detail page (media preview, metadata, transfers)                 | 5      |
| Liquidity Pools page (list, filters)                                 | 2      |
| Liquidity Pool detail page (summary + charts)                        | 5      |
| Search results page                                                  | 6      |
| Error states, loading skeletons, empty states                        | 3      |
| Polling, freshness indicators, responsive layout                     | 2      |
| **Subtotal**                                                         | **63** |

#### F. Testing

| Task                                                        | Days   |
| ----------------------------------------------------------- | ------ |
| Unit tests — API endpoints (cargo test)                     | 8      |
| Unit tests — XDR parsing + ingestion correctness            | 7      |
| Integration tests — end-to-end (ingestion → API → frontend) | 5      |
| Load testing (1M baseline scenario)                         | 4      |
| Security audit (OWASP Top 10)                               | 3      |
| Bug fixing + stabilization buffer                           | 15     |
| **Subtotal**                                                | **42** |

### 7.2 Summary

| Project Part                                 | Days        |
| -------------------------------------------- | ----------- |
| A. Design                                    | 35–40       |
| B. AWS Architecture + Galexie Infrastructure | 32          |
| C. Data Ingestion Pipeline                   | 25          |
| D. Core API Endpoints                        | 62          |
| E. Frontend Components + Integration         | 63          |
| F. Testing                                   | 42          |
| **Total (incl. design)**                     | **259–264** |

### 7.3 Cost Estimation (AWS, monthly)

#### Low Traffic (1M requests/month)

| Service                    | Configuration                                                                                                                               | Monthly Cost          |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | --------------------- |
| ECS Fargate — Galexie      | 1 vCPU / 2 GB RAM, continuous                                                                                                               | ~$36                  |
| Hetzner Server Auction     | `ch-prod-01` — 128 GB DDR4 ECC, 2× 1.92 TB U.2 NVMe RAID 1 (source: [ADR 0047](../../lore/2-adrs/0047_clickhouse-primary-api-datastore.md)) | ~€60                  |
| Hetzner BX21 Storage Box   | Borg backup target                                                                                                                          | estimate — not costed |
| API Gateway                | 1M requests + 0.5 GB cache                                                                                                                  | ~$4                   |
| Lambda — API handlers      | 800K invocations, 512 MB ARM                                                                                                                | ~$5                   |
| Lambda — Ingestion workers | ~500K invocations (Ledger Processor)                                                                                                        | ~$10                  |
| CloudFront                 | 10 GB transfer                                                                                                                              | ~$5                   |
| S3                         | Ledger XDR files (no auto-deletion, grows over time)                                                                                        | ~$5+                  |
| CloudWatch + X-Ray         | Logs, metrics, tracing                                                                                                                      | ~$20                  |
| Secrets Manager + Route 53 | Credentials + DNS                                                                                                                           | ~$10                  |

> **The RDS ($175 + $115) and NAT Gateway ($40) rows are gone, not moved.** Neither was
> ever deployed in production: RDS was superseded by ADR 0047 before cutover, and the NAT
> Gateway went away when task 0239 took the Lambdas out of the VPC. Dropping ~$330/month
> of AWS spend for ~€60 of Hetzner was the stated point of ADR 0047.
>
> No current total is given here on purpose — the surviving AWS figures are the original
> pre-pivot estimates and have not been checked against a bill. Treat every number in
> this table as an **estimate** except the Hetzner line, which is sourced.

#### Scaling Path to High Traffic (10M requests/month)

| Change                                              | Trigger                               | Added Cost                    |
| --------------------------------------------------- | ------------------------------------- | ----------------------------- |
| Add Lambda provisioned concurrency (5 instances)    | >2 req/s avg                          | +$75                          |
| Add a second ClickHouse node (replicated MergeTree) | CH CPU or disk sustained near ceiling | ~+€60 (deferred per ADR 0047) |
| API Gateway + Lambda growth                         | Proportional                          | +$30                          |
| CloudFront / data transfer growth                   | Proportional                          | +$20                          |

> The RDS read-replica / Multi-AZ rows this table used to carry described scaling levers
> for a database that was never deployed. ClickHouse on a dedicated box scales
> **vertically first** — the levers are a bigger auction server or a replicated second
> node, not a managed read replica. No total is quoted: see the note on the table above.

### 7.4 Three-Milestone Delivery Plan

#### Deliverable 1 — Indexing Pipeline & Core Infrastructure

> **Note (2026-05-20, corrected 2026-07-22):** Per [ADR 0047](../../lore/2-adrs/0047_clickhouse-primary-api-datastore.md),
> the prod datastore for API reads is ClickHouse on Hetzner, not RDS PostgreSQL.
> Acceptance criteria #2 and #3 below reference "ClickHouse" accordingly.
>
> ~~RDS retirement is scheduled in task 0239 Phase 6.~~ **There was no retirement.** > [Task 0239](../../lore/1-tasks/archive/0239_FEATURE_aws-side-cutover-mtls-to-hetzner.md)
> completed with Phase 6 satisfied **vacuously** — no production RDS was ever deployed.
> The store-identity / data-model prose was swept to ClickHouse in task 0244; the §3
> Infrastructure topology and §7 Scaling Model / cost tables were swept in task 0248
> (2026-07-22). What follows below is the delivery plan as originally written and is
> retained as a record of scope, not as a description of the deployment.
>
> **Note (2026-05-27):** The Ledger Processor trigger mechanism was reworked
> from per-S3-event Lambda invocations to an **SQS doorbell + ClickHouse-cursor
> reconcile** in [task 0241](../../lore/1-tasks/archive/0241_FEATURE_indexer-hard-swap-pg-to-ch-and-cutover-runbook.md).
> The PG-era migration + partition Lambdas (`crates/db-migrate`,
> `crates/db-partition-mgmt`) and their CDK stacks were removed in the same
> task; ClickHouse applies its schema box-side via the `db-clickhouse-init`
> sidecar and auto-creates parts via `PARTITION BY intDiv(sequence, 500000)`.

Galexie ECS Fargate task running on mainnet, writing `LedgerCloseMeta` XDR files to S3
every ~5–6 seconds. Lambda Ledger Processor woken by an **SQS doorbell** (S3
`ObjectCreated` notifications publish to the `{env}-ledger-events` SNS topic,
which fans out to the `ledger-ingest` queue via a `rawMessageDelivery`
subscription — and to a second tenant, prices-api, on its own queue, task 0306;
`batchSize 1`, `ReportBatchItemFailures`); each invocation reconciles forward from
`max(sequence) + 1` in ClickHouse oldest-first and persists the contiguous run until
the next ledger is absent on S3 (gap) or the 540 s time budget is reached.
`reservedConcurrentExecutions = 1` guarantees ascending, gapless ordering without
needing an SQS FIFO queue; per-invocation state lives in ClickHouse, so the Lambda
itself is stateless and freely re-runnable. The Lambda parses and writes
ledgers, transactions, operations, accounts, Soroban invocations, and CAP-67 events to a
dedicated **ClickHouse on Hetzner** database (`ch-prod-01`, single-node MergeTree, mTLS
behind Caddy). Historical backfill from Soroban mainnet activation ledger (late 2023)
delivered via FREEZE + rsync + ATTACH PART transport per
[ADR 0045](../../lore/2-adrs/0045_clickhouse-local-backfill-then-mirror-to-hetzner-via-freeze-rsync-attach.md).
Rust API scaffolding with core modules (axum + utoipa). OpenAPI specification. AWS CDK
infrastructure-as-code (AWS side) plus an Ansible playbook for the Hetzner database host;
both halves are run from an operator's machine (`cdk deploy` and
`ansible-playbook`) against clean environments with no manual one-off steps.
CloudWatch dashboards and ingestion lag alarms.

**Acceptance criteria:**

1. S3 bucket contains consecutive `LedgerCloseMeta` files with timestamps matching
   mainnet ledger close times
2. **ClickHouse on Hetzner** `ledgers` table contains all ledgers from backfill start
   through current tip with no gaps
3. **ClickHouse on Hetzner** `soroban_events` table contains full-content rows for CAP-67
   events in known Soroswap/Aquarius/Phoenix transactions (spot-checked by
   transaction hashes); decoded events are confirmed by fetching the
   corresponding `.xdr.zst` from the public archive and re-expanding via
   `xdr_parser::extract_events`
4. `cdk deploy` (AWS side) + `ansible-playbook` (Hetzner side) from clean environments
   produces the full working stack with no manual steps
5. CloudWatch dashboard accessible; Galexie lag alarm fires correctly in staging

**Budget: $26,240 (20% of total)**

---

#### Deliverable 2 — Complete API + Frontend

All REST API endpoints live and serving mainnet data: transactions (list + detail),
ledgers (list + detail), accounts (detail + history), contracts (detail + invocations +
events), assets, NFTs, liquidity pools, search (exact match + prefix). React SPA deployed
via CloudFront with all pages. Rate limiting and response
caching configured on API Gateway.

**Acceptance criteria:**

1. All API endpoints return schema-valid responses for mainnet entity IDs provided by
   the reviewer
2. Soroban invocations on Contract Detail page show function name, arguments, and return
   value (not raw XDR) for at least 3 known contract transactions
3. CAP-67 events appear on Transaction Detail page under Events tab with decoded topics
   and data fields (not raw XDR)
4. Global search redirects to correct detail page for an exact transaction hash, account
   ID, and contract ID
5. React frontend publicly accessible at staging URL; all pages render live mainnet data

**Budget: $39,360 (30% of total)**

---

#### Deliverable 3 — Mainnet Launch

Production deployment on mainnet at public URL. Unit and integration tests covering XDR
parsing correctness and API endpoint responses. Load test
results documented (1M baseline, 10M stress). Security audit checklist (OWASP Top 10,
IAM least-privilege, no publicly reachable database endpoint). Monitoring dashboards and alerting active
and accessible to Stellar team. Full API reference documentation published. GitHub
repository made public. Professional user testing completed. 7-day post-launch monitoring
report.

**Acceptance criteria:**

1. Block explorer publicly accessible at production URL, showing live mainnet data with
   ledger sequences matching network tip within 30 seconds
2. GitHub repository public; `cdk deploy` from README works in a fresh AWS account
3. CloudWatch dashboard accessible to Stellar team (read-only IAM role); all alarms OK;
   Galexie ingestion lag <30 s from network tip
4. Load test report: p95 <200 ms at 1M requests/month equivalent; error rate <0.1%
5. Security checklist signed off: no wildcard IAM, WAF/throttling active on public
   ingress, ClickHouse reachable only through Caddy with a client certificate whose CN
   is on the allow-list (no anonymous public endpoint), weekly off-box Borg backup
   (Sunday 03:30 UTC, `--keep-weekly 4`) with a restore procedure drill-tested
   locally end-to-end, disk-level redundancy via mdadm RAID 1, the `stellar-ledger-data` bucket
   with SSE-S3 (AES256), all secrets in Secrets Manager, all
   API inputs validated

   > Deliberate deltas from the original AWS-shaped checklist, recorded rather than
   > quietly dropped: **there is no PITR** (weekly Borg only — RPO up to 7 days), and
   > **no deletion protection** in the RDS sense, because the store is a self-hosted box.
   > Both were properties of a managed service that was never adopted.
   >
   > **"WAF/throttling active on public ingress" is satisfied without AWS WAF.** Both
   > AWS WAF WebACLs were dropped ([ADR 0048](../../lore/2-adrs/0048_cloudflare-edge-over-aws-waf.md),
   > task 0302). What holds the criterion: API Gateway throttling (`50` rps / `100`
   > burst) plus usage-plan limits on every public route, and the Cloudflare edge —
   > managed rules, DDoS, Managed Challenge, rate limiting — on the API hostname, with
   > the origin locked to Cloudflare. The CloudFront frontend distribution carries no
   > edge filter; it serves static, edge-cached files from a private S3 origin, so the
   > managed rule groups had nothing to protect there.
   >
   > This wording is an expanded restatement of the criterion, not the approved text —
   > the approved wording is the arbiter.
   >
   > **The backup clause was narrowed to what is demonstrable.** It previously read
   > "weekly Borg backup **verified restorable**", which conflated two different
   > things. Per [`docs/backups.md`](../backups.md): the restore procedure has been
   > drill-tested locally end-to-end, but a real restore **from the Hetzner Storage
   > Box has never been performed** — that would be the operator's first live
   > exercise. "Verified restorable" would claim the second on the strength of the
   > first. RPO is up to 7 days, as recorded above.

6. 7-day post-launch monitoring report: uptime %, API error rate, p95 latency, Galexie
   ingestion lag per day

**Budget: $52,480 (40% of total + professional user testing)**

### 7.5 Risk Areas

- **XDR schema evolution** — new CAPs may change `LedgerCloseMeta` structure. Mitigated
  by tracking Stellar Core releases; protocol upgrades are well-announced.
- **Frontend blockchain learning curve** — transaction detail tree view requires deep
  understanding of Stellar data structures. Mitigated by mock API responses built in
  parallel with backend development.
- **Backfill volume** — indexing from Soroban activation to present will produce hundreds
  of GB of data. Mitigated by running backfill as a background task from day 1 and
  launching with recent history if backfill is not complete at milestone 1.
- **NFT and Liquidity Pool data** — Stellar's NFT ecosystem is nascent; LP chart data
  requires aggregation. Mitigated by building these pages last; graceful empty states
  designed from the start.
