# Stellar Block Explorer — Technical Design (Post-Review)

> This document supersedes `soroban-first-block-explorer.md`. It incorporates changes required by reviewer
> feedback: the indexing layer is rebuilt around self-hosted Galexie (direct ledger
> processing) replacing the deprecated Horizon API, the block explorer now owns its own
> database, and component ownership is explicitly demarcated.

A production-grade, Soroban-first block explorer for the Stellar network. The system
prioritizes **human-readable transaction display** and first-class Soroban smart contract
support. The frontend communicates exclusively with a custom NestJS REST API, which sources
chain data from the block explorer's own PostgreSQL database — populated by a Galexie-based
ingestion pipeline that processes `LedgerCloseMeta` XDR directly from the Stellar network.

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

### 1.3 Routes and Pages

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
- Balances — native XLM balance and trustline/token balances
- Recent transactions — paginated table of transactions involving this account

#### Tokens (`/tokens`)

List of all known tokens (classic Stellar assets and Soroban token contracts).

- Token table — asset code, issuer / contract ID, type (classic / SAC / Soroban), total
  supply, holder count
- Filters — type (classic, SAC, Soroban), asset code search
- Cursor-based pagination controls

#### Token (`/tokens/:id`)

Single token detail view.

- Token summary — asset code, issuer or contract ID (copyable), type badge, total supply,
  holder count, deployed at ledger (if Soroban)
- Metadata — name, description, icon (if available), domain/home page
- Latest transactions — paginated table of recent transactions involving this token

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

#### NFT (`/nfts/:id`)

Single NFT overview.

- NFT summary — name, identifier/token ID, collection name, contract ID (link), owner
  account (link)
- Media preview — image, video, or other media associated with the NFT
- Metadata — full attribute list (traits, properties)
- Transfer history — table of ownership changes

#### Liquidity Pools (`/liquidity-pools`)

Paginated table of all liquidity pools.

- Pool table — pool ID (truncated), asset pair (e.g. XLM/USDC), total shares, reserves
  per asset, fee percentage
- Filters — asset pair, minimum TVL
- Cursor-based pagination controls

#### Liquidity Pool (`/liquidity-pools/:id`)

- Pool summary — pool ID (full, copyable), asset pair, fee percentage, total shares,
  reserves per asset
- Charts — TVL over time, volume over time, fee revenue
- Pool participants — table of liquidity providers and their share
- Recent transactions — deposits, withdrawals, and trades involving this pool

#### Search Results (`/search?q=`)

Generic search across all entity types. For exact matches (transaction hash, contract ID,
account ID), redirects directly to the detail page. Otherwise displays grouped results.

- Search input — pre-filled with current query, allows refinement
- Results grouped by type — transactions, contracts, tokens, accounts, NFTs, liquidity
  pools (with type headers and counts)
- Each result row — identifier (linked), type badge, brief context
- Empty state — "No results found" with suggestions

### 1.4 Shared UI Elements

Present across all pages:

- **Header** — logo, global search bar, network indicator (mainnet/testnet)
- **Navigation** — links to home, transactions, ledgers, tokens, contracts, NFTs,
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

The backend is a NestJS application running on AWS Lambda behind API Gateway. It is a
REST API. The backend does not perform chain indexing; it reads from the block explorer's
own PostgreSQL database, which is populated by the Galexie-based ingestion pipeline.

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

### 2.2 API Responsibilities and Boundaries

The backend serves data from the block explorer's own database, adding:

- **Data normalization** — transforms raw indexed records into a consistent,
  frontend-friendly format (e.g. flattening nested fields, attaching human-readable
  operation summaries and event interpretations)
- **Soroban enrichment** — decorates contract invocations with metadata, function names,
  and structured interpretations stored at ingestion time
- **Search** — unified search across transaction hashes, account IDs, contract IDs, token
  identifiers, NFT identifiers, pool IDs, and indexed metadata using PostgreSQL full-text
  indexes
- **Raw XDR on demand** — the `envelope_xdr`, `result_xdr`, and `result_meta_xdr` fields
  are stored verbatim for advanced inspection; the backend returns the first two in the
  advanced transaction view, decodes the raw payloads on request using
  `@stellar/stellar-sdk`, and can serve a stored `operation_tree` for transaction-detail
  debugging sections

The backend does **not** call Horizon or any external chain API. All chain data lives in
the block explorer's RDS.

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

#### Ledgers

**`GET /ledgers`** — Paginated list of ledgers.

**`GET /ledgers/:sequence`** — Ledger detail including transaction count and linked
transactions.

#### Accounts

**`GET /accounts/:account_id`** — Account detail: current balances, sequence number,
and first/last seen ledger.

**`GET /accounts/:account_id/transactions`** — Paginated transactions involving this
account.

#### Tokens

**`GET /tokens`** — Paginated list of tokens (classic assets + Soroban token contracts).
Query params: `limit`, `cursor`, `filter[type]` (classic/sac/soroban), `filter[code]`.

**`GET /tokens/:id`** — Token detail: asset code, issuer/contract, type, supply, holder
count, metadata.

**`GET /tokens/:id/transactions`** — Paginated transactions involving this token.

#### Contracts

**`GET /contracts/:contract_id`** — Contract metadata, deployer, WASM hash, stats.

**`GET /contracts/:contract_id/interface`** — Public function signatures (names, parameter
types, return types).

**`GET /contracts/:contract_id/invocations`** — Paginated list of contract invocations.

**`GET /contracts/:contract_id/events`** — Paginated list of contract events.

#### NFTs

**`GET /nfts`** — Paginated list of NFTs. Query params: `limit`, `cursor`,
`filter[collection]`, `filter[contract_id]`.

**`GET /nfts/:id`** — NFT detail: name, token ID, collection, contract, owner, metadata,
media URL.

**`GET /nfts/:id/transfers`** — Transfer history for a single NFT.

#### Liquidity Pools

**`GET /liquidity-pools`** — Paginated list of pools. Query params: `limit`, `cursor`,
`filter[assets]`, `filter[min_tvl]`.

**`GET /liquidity-pools/:id`** — Pool detail: asset pair, fee, reserves, total shares, TVL.

**`GET /liquidity-pools/:id/transactions`** — Deposits, withdrawals, and trades for this
pool.

**`GET /liquidity-pools/:id/chart`** — Time-series data for TVL, volume, and fee revenue.
Query params: `interval` (1h/1d/1w), `from`, `to`.

#### Search

**`GET /search?q=&type=transaction,contract,token,account,nft,pool`** — Generic search
across all entity types. Uses prefix/exact matching on hashes, account IDs, contract IDs,
asset codes, pool IDs, and NFT identifiers. Full-text search on metadata via
`tsvector`/`tsquery` and GIN indexes where entity metadata is indexed.

### 2.4 Caching Strategy

Caching operates at two levels:

- **API Gateway response caching** — responses for immutable data (historical
  transactions, closed ledgers) are cached with long TTLs at the API ingress layer. Mutable
  data (recent transactions, network stats) uses short TTLs (5–15 seconds). CloudFront is
  used for static frontend assets and is not assumed to be the primary cache layer for API
  responses in the initial topology.
- **Backend in-memory caching** — frequently accessed reference data (contract metadata,
  network stats) is cached in the Lambda execution environment with TTLs of 30–60 seconds
  to reduce database round-trips.

### 2.5 Fault Tolerance

- **Ingestion lag** — if the Galexie pipeline falls behind, the API continues serving
  data from the database with a freshness indicator showing the highest indexed ledger
  sequence. A CloudWatch alarm fires at >60 s lag.
- **Lambda cold starts** — mitigated via ARM/Graviton2 runtime and provisioned concurrency
  at higher traffic tiers.
- **Database connection pooling** — RDS Proxy manages connection pools to prevent
  exhaustion under burst traffic.

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
│  │ RDS PostgreSQL (block explorer's own schema)         │                     │
│  │ ledgers · transactions · operations · accounts       │                     │
│  │ contracts · soroban_invocations · events · tokens    │                     │
│  │ nfts · liquidity_pools · liquidity_pool_snapshots    │                     │
│  └──────────────────────────┬───────────────────────────┘                     │
│                             │                                                 │
│  API LAYER                  │                                                 │
│  ┌──────────────────────────▼──────────┐  ┌────────────────────────────────┐  │
│  │ API Gateway → Lambda (NestJS)       │  │ CloudFront CDN                 │  │
│  │ REST, throttling, WAF               │  │ React SPA + static assets      │  │
│  └─────────────────────────────────────┘  └────────────────────────────────┘  │
└───────────────────────────────────────────────────────────────────────────────┘

Connections:
  Stellar network peers → Galexie (Captive Core, live ledger stream)
  Stellar history archives → Galexie backfill task (one-time, batch)
  Galexie → S3 (LedgerCloseMeta XDR files)
  S3 PutObject event → Lambda Ledger Processor
  Lambda Ledger Processor → RDS (write)
  Lambda NestJS API → RDS (read)
  React SPA → API Gateway → Lambda NestJS API
```

### 3.2 Deployment Model

All infrastructure runs on AWS, deployed to a **dedicated AWS sub-account owned by Rumble
Fish**. At launch the system is deployed in a single Availability Zone (`us-east-1a`),
expanding to multi-AZ when SLA requirements demand it.

```
┌─ VPC — us-east-1a ──────────────────────────────────────────────────────────┐
│                                                                             │
│  ┌─ Public Subnet ───────────────────────────────────────────────────────┐  │
│  │  CloudFront CDN                 API Gateway                           │  │
│  └────────┬───────────────────────────┬──────────────────────────────────┘  │
│           │                           │                                     │
│  ┌─ Private Subnet ──────────────────────────────────────────────────────┐  │
│  │        │                           ▼                                  │  │
│  │        │                  Lambda (NestJS API)                         │  │
│  │        │                  Lambda (Ledger Processor)                   │  │
│  │        │                  Lambda (Event Interpreter)                  │  │
│  │        │                           │                                  │  │
│  │        │              ┌────────────┴────────────┐                     │  │
│  │        │              │ RDS PostgreSQL           │                     │  │
│  │        │              │ (block explorer schema)  │                     │  │
│  │        │              └─────────────────────────┘                     │  │
│  └────────┼──────────────────────────────────────────────────────────────┘  │
└───────────┼─────────────────────────────────────────────────────────────────┘
            │
  ECS Fargate (Galexie) runs in the same VPC, writing to S3 via VPC endpoint
  Route 53 ──> CloudFront CDN
  Lambda ····> Secrets Manager (credentials)
  Lambda ····> CloudWatch / X-Ray (monitoring)
```

### 3.3 Component Ownership

**Hosted by Rumble Fish (AWS sub-account):**

| Component                       | Service                            | Role                                                                   |
| ------------------------------- | ---------------------------------- | ---------------------------------------------------------------------- |
| Galexie process                 | ECS Fargate (1 task, continuous)   | Streams live ledger data from Stellar network to S3                    |
| Historical backfill task        | ECS Fargate (batch, one-time)      | Processes history archives to backfill from Soroban mainnet activation |
| S3 bucket `stellar-ledger-data` | AWS S3                             | Receives `LedgerCloseMeta` XDR files; triggers Ledger Processor        |
| Lambda — Ledger Processor       | AWS Lambda (S3 event-driven)       | Parses XDR; writes explorer records and derived state to RDS           |
| Lambda — Event Interpreter      | AWS Lambda (EventBridge, 5 min)    | Post-processes recent events to generate human-readable summaries      |
| Lambda — NestJS API handlers    | AWS Lambda (per API Gateway route) | Serves all public API requests                                         |
| RDS PostgreSQL                  | AWS RDS (db.r6g.large, Single-AZ)  | Block explorer database                                                |
| API Gateway                     | AWS API Gateway                    | REST API, throttling, request validation, response caching             |
| AWS WAF                         | AWS WAF                            | Managed rules and abuse protection for public ingress                  |
| CloudFront CDN                  | AWS CloudFront                     | Serves React frontend                                                  |
| Swagger UI                      | NestJS `/api-docs` endpoint        | OpenAPI spec + interactive documentation                               |
| EventBridge Scheduler           | AWS EventBridge                    | Cron triggers for background workers                                   |
| Secrets Manager                 | AWS Secrets Manager                | DB credentials, non-browser integration keys                           |
| CloudWatch + X-Ray              | AWS CloudWatch                     | Logs, metrics, alarms, distributed tracing                             |
| CI/CD pipeline                  | GitHub Actions → AWS CDK           | Infrastructure-as-code deploy                                          |

**External services consumed (read-only):**

| External service                             | Purpose                        | Failure impact                                       |
| -------------------------------------------- | ------------------------------ | ---------------------------------------------------- |
| Stellar network peers (Galexie Captive Core) | Live ledger data source        | Critical — mitigated by connecting to multiple peers |
| Stellar public history archives              | Historical backfill (one-time) | Non-critical after backfill completes                |

No external APIs (Horizon, Soroswap, Aquarius, Soroban RPC) are required for core
functionality. All data flows from the canonical ledger.

Public browser traffic is anonymous and read-only. The SPA must not embed API keys or
shared secrets. Abuse control for public traffic is enforced at the ingress layer through
API Gateway throttling/request validation and AWS WAF. If API keys are later enabled, they
are reserved for trusted non-browser consumers and are not required for normal explorer
browsing.

### 3.4 Tech Stack

| Component     | Technology                       | Purpose                                                   |
| ------------- | -------------------------------- | --------------------------------------------------------- |
| Ingestion     | Galexie (ECS Fargate)            | Streams `LedgerCloseMeta` XDR from Stellar network to S3  |
| XDR parsing   | `@stellar/stellar-sdk` (Node.js) | Deserializes all XDR types in Ledger Processor Lambda     |
| API Framework | NestJS / TypeScript              | Modular REST API                                          |
| Compute       | AWS Lambda (ARM/Graviton2)       | Serverless; auto-scaling                                  |
| Gateway       | AWS API Gateway                  | Request routing, throttling, validation, response caching |
| Edge Security | AWS WAF                          | Managed rules, IP reputation, abuse protection            |
| Database      | RDS PostgreSQL 16                | Block explorer schema with native range partitioning      |
| CDN           | CloudFront                       | Static asset delivery for frontend                        |
| DNS           | Route 53                         | Domain management                                         |
| Monitoring    | CloudWatch + X-Ray               | Logging, distributed tracing, alarms                      |
| Secrets       | Secrets Manager                  | Database credentials, non-browser integration keys        |
| IaC           | AWS CDK (TypeScript)             | All infrastructure defined as code                        |
| CI/CD         | GitHub Actions → `cdk deploy`    | Automated deployment on merge to main                     |

### 3.5 Environments

| Environment     | Purpose                   | Database                    |
| --------------- | ------------------------- | --------------------------- |
| **Development** | Local and CI development  | Local PostgreSQL            |
| **Staging**     | Pre-production validation | Separate RDS (testnet data) |
| **Production**  | Live service              | Mainnet RDS                 |

Production is the public traffic baseline. Staging preserves the same topology and failure
model, but uses lower concurrency/throttling budgets, smaller caches, shorter operational
retention, and tighter access controls so pre-production validation does not carry full
production cost. The staging web frontend should not be publicly open; it is expected to be
protected by password-based access at the edge layer. Production durability and security
baselines explicitly include automated RDS backups with point-in-time recovery, deletion
protection on the production database, KMS-backed encryption for RDS and S3, and TLS on
public ingress.

### 3.6 Scalability

| Component            | Mechanism                                    | Trigger              |
| -------------------- | -------------------------------------------- | -------------------- |
| **API**              | Lambda auto-scale (up to 50 concurrent)      | On-demand            |
| **Ledger Processor** | Lambda auto-scale (S3 event-driven)          | Per ledger file      |
| **PostgreSQL**       | RDS Proxy for connection pooling             | Default              |
|                      | Materialized views for aggregated statistics | Default              |
|                      | Add read replica                             | Primary CPU > 60%    |
| **CDN**              | CloudFront scales automatically              | N/A                  |
| **Multi-AZ**         | Expand VPC + enable RDS Multi-AZ             | SLA > 99.9% required |

### 3.7 Monitoring and Alerting

| Alarm                       | Threshold                                        | Action               |
| --------------------------- | ------------------------------------------------ | -------------------- |
| Galexie ingestion lag       | S3 file timestamp >60 s behind ledger close time | SNS → Slack/email    |
| Ledger Processor error rate | >1% of Lambda invocations in error               | SNS → Slack/email    |
| RDS CPU                     | >70% sustained for 5 min                         | SNS → on-call        |
| RDS free storage            | <20% remaining                                   | SNS → expand storage |
| API Gateway 5xx rate        | >0.5% of requests                                | SNS → Slack/email    |

The thresholds above are the production baseline. Staging may use lower-volume alerting,
tighter cost ceilings, and shorter retention so long as the same alarm categories remain
represented before production rollout.

CloudWatch dashboards expose: Galexie S3 file freshness, Ledger Processor duration and
error rate, API latency (p50/p95/p99), RDS CPU/connections, and highest indexed ledger
sequence vs. network tip.

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
│                    .xdr.zstd     │
└──────────────┬───────────────────┘
               │ S3 PutObject event notification
               ▼
┌─────────────────────────────────────────────────────────┐
│  Lambda "Ledger Processor"  (event-driven, per file)    │
│  1. Download + decompress XDR                           │
│  2. Parse LedgerCloseMeta via @stellar/stellar-sdk      │
│  3. Extract ledger header (sequence, close_at, proto)   │
│  4. Extract all transactions: hash, source, fee,        │
│     success/failure, envelope XDR, result XDR           │
│  5. Extract operations: type, details per operation     │
│  6. Extract Soroban invocations (INVOKE_HOST_FUNCTION): │
│     contract ID, function name, args, return value      │
│  7. Extract CAP-67 events (SorobanTransactionMeta       │
│     .events): all contract events in one stream         │
│  8. Extract contract deployments (new C-addresses,      │
│     WASM hashes) from LedgerEntryChanges                │
│  9. Extract account state snapshots (sequence, balances,│
│     home_domain) from LedgerEntryChanges                │
│ 10. Detect token contracts (SEP-41), NFT contracts,     │
│     liquidity pools from deployment events              │
│ 11. Write all above to RDS PostgreSQL                   │
└─────────────────────────────────────────────────────────┘
               │
               ▼
       RDS PostgreSQL (block explorer schema — Section 6)
```

### 4.2 What `LedgerCloseMeta` Contains

The `LedgerCloseMeta` XDR produced by Galexie contains the complete ledger close.
Everything a block explorer needs is present; no external API is required.

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

For historical data, a separate ECS Fargate task reads from Stellar's **public history
archives** (the same archives that Horizon used for `db reingest`). It writes
`LedgerCloseMeta` files in the same format to the same S3 bucket, triggering the same
Ledger Processor Lambda. No separate code path is required.

- **Scope:** from Soroban mainnet activation ledger (late 2023) to the present
- **Parallelism:** backfill runs in configurable ledger-range batches. Batches may execute
  in parallel only when they own non-overlapping ledger ranges and preserve deterministic
  replay semantics
- **Timing:** runs as a one-time batch during Phase 1 (Deliverable 1); live ingestion
  continues in parallel, and live-derived state remains authoritative for the newest
  ledgers

### 4.4 Background Workers

| Worker                | Trigger                     | Role                                                                                                 |
| --------------------- | --------------------------- | ---------------------------------------------------------------------------------------------------- |
| **Ledger Processor**  | S3 PutObject (~every 5–6 s) | Primary ingestion — parses XDR, writes all chain data to RDS                                         |
| **Event Interpreter** | EventBridge rate(5 min)     | Post-processes new events to generate human-readable summaries (swap, transfer, mint, burn patterns) |

### 4.5 Operational Characteristics

**Normal operation (live):**

```
Galexie (ECS Fargate) → S3 (~5-6 s per ledger)
                      → Lambda Ledger Processor (~<10 s from ledger close to DB write)
```

**Recovery from Galexie restart:** Galexie is checkpoint-aware. On restart it reads the
last exported ledger sequence and resumes from there. No manual intervention required.

**Recovery from Ledger Processor failure:** S3 PutObject event notifications are retried
by Lambda automatically. For permanent failures, the file remains in S3 and can be
replayed by re-triggering the Lambda with the S3 key.

**Replay artifact retention:** the `stellar-ledger-data` bucket retains files indefinitely
(ADR 0006). No automatic deletion. This supports replay and post-incident validation at any
point. Lifecycle rules can be added later if storage costs grow. Previously planned as 30 days production / 7
days. Lifecycle expiration happens only after that minimum replay window.

**Idempotency and ordering:** ledger sequence is the canonical ordering key. Processing is
replay-safe: immutable ledger-scoped writes happen transactionally per ledger, and
reprocessing the same ledger replaces or de-duplicates that ledger's immutable rows rather
than creating duplicates. Derived-state upserts (`accounts`, `tokens`, `nfts`,
`liquidity_pools`) apply only when the incoming ledger sequence is newer than or equal to
the stored watermark (`last_seen_ledger` / `last_updated_ledger`), so an older backfill
batch cannot overwrite fresher live state.

**Schema migrations:** versioned, managed via AWS CDK and run as part of the CI/CD
pipeline before deploying new Lambda code.

**Protocol upgrades:** when Stellar introduces a new CAP that changes `LedgerCloseMeta`
structure, we update `@stellar/stellar-sdk` XDR types. Protocol upgrades are infrequent
and well-announced in advance.

**Open-source re-deployability:** the full CDK stack is public; Stellar or any third party
can fork the repository and deploy the entire system in a fresh AWS account.

---

## 5. XDR Parsing

### 5.1 Parsing Strategy

XDR parsing happens in two places:

- **Ledger Processor Lambda (at ingestion time):** the primary parsing path. Every ledger's
  `LedgerCloseMeta` is fully deserialized using `@stellar/stellar-sdk` XDR types.
  Structured results are written to RDS. The frontend receives pre-decoded data for all
  normal operations.

- **NestJS API (on request):** the raw `envelope_xdr`, `result_xdr`, and `result_meta_xdr`
  strings are stored verbatim in RDS. The API returns `envelope_xdr` and `result_xdr` to
  the frontend for the advanced view, and can decode all three on request using
  `@stellar/stellar-sdk` to serve additional structured fields or validate the stored
  invocation tree when needed.

### 5.2 Data Extracted at Ingestion (Ledger Processor)

**From `LedgerHeader`:**

- `sequence`, `closeTime`, `protocolVersion`, `baseFee`, `txSetResultHash`

**From `TransactionEnvelope` + `TransactionResult`:**

- `hash` (computed by hashing the envelope XDR), `sourceAccount`, `feeCharged`,
  `successful`, `resultCode`
- Raw `envelopeXdr`, `resultXdr`, and `resultMetaXdr` stored verbatim for advanced view
  and transaction-tree debugging

**From `OperationMeta` per transaction:**

- Operation `type`, structured `details` JSONB (type-specific fields)
- For `INVOKE_HOST_FUNCTION`: `contractId`, `functionName`, `functionArgs` (decoded
  `ScVal`), `returnValue` (decoded `ScVal`)

**From `SorobanTransactionMeta.events`:**

- `eventType` (contract/system/diagnostic), `contractId`, `topics` (decoded `ScVal[]`),
  `data` (decoded `ScVal`)
- Known NFT contract patterns interpreted into derived NFT ownership and metadata updates

**From `LedgerEntryChanges`:**

- Contract deployments: `contractId`, `wasmHash`, `deployerAccount`
- Account changes and account-state snapshots (`sequence_number`, `balances`, `home_domain`)
- Liquidity pool state changes (`poolId`, asset pair, reserves, total shares)

### 5.3 Soroban-Specific Handling

- **CAP-67 events** are decoded from `SorobanTransactionMeta.events` at ingestion time
  and stored in the `soroban_events` table as structured JSONB. The API serves them
  decoded — the frontend does not need to handle raw XDR for events.
- **Return values** — the return value of `invokeHostFunction` is an XDR `ScVal` decoded
  to a typed value (integer, string, address, bytes, map, etc.) and stored in the
  `soroban_invocations` table.
- **Invocation tree** — complex transactions with nested contract-to-contract calls have
  their full invocation hierarchy decoded from `result_meta_xdr`, stored in
  `transactions.operation_tree`, and served directly to the transaction detail tree view.
  The raw `result_meta_xdr` is preserved for advanced decode/debug use.
- **Contract interface** — function signatures (names, parameter types) are extracted from
  the contract WASM at deployment time and stored inside `soroban_contracts.metadata`.

### 5.4 Error Handling

- **Malformed XDR** — if `fromXDR()` throws, the Ledger Processor logs the error with the
  transaction hash, stores the raw XDR verbatim, and marks the record with a
  `parse_error` flag. The API returns the raw XDR to the frontend with a decode-failure
  indicator. The transaction is still displayed with all available non-XDR fields.
- **Unknown operation types** — new protocol versions may introduce operation types not
  yet supported by the SDK. These are rendered as "Unknown operation" with raw XDR shown,
  and a CloudWatch alarm is raised to trigger an SDK update.

---

## 6. Database Schema

The block explorer owns its full PostgreSQL schema. All chain data is stored here;
there is no dependency on an external database.

High-volume Soroban activity tables and liquidity-pool time series use **native range
partitioning by month** for efficient time-range queries and instant partition drops. The
`operations` table remains partitioned separately by `transaction_id` in the current
schema.

### 6.1 Ledgers

```sql
CREATE TABLE ledgers (
    sequence          BIGINT PRIMARY KEY,
    hash              VARCHAR(64) UNIQUE NOT NULL,
    closed_at         TIMESTAMPTZ NOT NULL,
    protocol_version  INT NOT NULL,
    transaction_count INT NOT NULL,
    base_fee          BIGINT NOT NULL,
    INDEX idx_closed_at (closed_at DESC)
);
```

### 6.2 Transactions

```sql
CREATE TABLE transactions (
    id               BIGSERIAL PRIMARY KEY,
    hash             VARCHAR(64) UNIQUE NOT NULL,
    ledger_sequence  BIGINT REFERENCES ledgers(sequence),
    source_account   VARCHAR(56) NOT NULL,
    fee_charged      BIGINT NOT NULL,
    successful       BOOLEAN NOT NULL,
    result_code      VARCHAR(50),
    envelope_xdr     TEXT NOT NULL,
    result_xdr       TEXT NOT NULL,
    result_meta_xdr  TEXT,
    memo_type        VARCHAR(20),
    memo             TEXT,
    created_at       TIMESTAMPTZ NOT NULL,
    parse_error      BOOLEAN DEFAULT FALSE,
    operation_tree   JSONB,
    INDEX idx_hash (hash),
    INDEX idx_source (source_account, created_at DESC),
    INDEX idx_ledger (ledger_sequence)
);
```

### 6.3 Operations

```sql
CREATE TABLE operations (
    id              BIGSERIAL PRIMARY KEY,
    transaction_id  BIGINT REFERENCES transactions(id) ON DELETE CASCADE,
    type            VARCHAR(50) NOT NULL,
    details         JSONB NOT NULL,
    INDEX idx_tx (transaction_id),
    INDEX idx_details (details) USING GIN
) PARTITION BY RANGE (transaction_id);
```

### 6.4 Soroban Contracts

```sql
CREATE TABLE soroban_contracts (
    contract_id        VARCHAR(56) PRIMARY KEY,
    wasm_hash          VARCHAR(64),
    deployer_account   VARCHAR(56),
    deployed_at_ledger BIGINT REFERENCES ledgers(sequence),
    contract_type      VARCHAR(50),  -- 'token', 'dex', 'lending', 'nft', 'other'
    is_sac             BOOLEAN DEFAULT FALSE,
    metadata           JSONB,        -- explorer metadata incl. optional interface signatures
    search_vector      TSVECTOR GENERATED ALWAYS AS (
                           to_tsvector('english', coalesce(metadata->>'name', ''))
                       ) STORED,
    INDEX idx_type (contract_type),
    INDEX idx_search (search_vector) USING GIN
);
```

### 6.5 Soroban Invocations

```sql
CREATE TABLE soroban_invocations (
    id               BIGSERIAL PRIMARY KEY,
    transaction_id   BIGINT REFERENCES transactions(id) ON DELETE CASCADE,
    contract_id      VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    caller_account   VARCHAR(56),
    function_name    VARCHAR(100) NOT NULL,
    function_args    JSONB,
    return_value     JSONB,
    successful       BOOLEAN NOT NULL,
    ledger_sequence  BIGINT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    INDEX idx_contract (contract_id, created_at DESC),
    INDEX idx_function (contract_id, function_name)
) PARTITION BY RANGE (created_at);
```

### 6.6 Soroban Events (CAP-67)

```sql
CREATE TABLE soroban_events (
    id               BIGSERIAL PRIMARY KEY,
    transaction_id   BIGINT REFERENCES transactions(id) ON DELETE CASCADE,
    contract_id      VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    event_type       VARCHAR(20) NOT NULL,  -- 'contract', 'system', 'diagnostic'
    topics           JSONB NOT NULL,
    data             JSONB NOT NULL,
    ledger_sequence  BIGINT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    INDEX idx_contract (contract_id, created_at DESC),
    INDEX idx_topics (topics) USING GIN
) PARTITION BY RANGE (created_at);
```

### 6.7 Event Interpretations

```sql
CREATE TABLE event_interpretations (
    id                   BIGSERIAL PRIMARY KEY,
    event_id             BIGINT REFERENCES soroban_events(id) ON DELETE CASCADE,
    interpretation_type  VARCHAR(50) NOT NULL,  -- 'swap', 'transfer', 'mint', 'burn'
    human_readable       TEXT NOT NULL,
    structured_data      JSONB NOT NULL,
    INDEX idx_type (interpretation_type)
);
```

### 6.8 Tokens

```sql
CREATE TABLE tokens (
    id               SERIAL PRIMARY KEY,
    asset_type       VARCHAR(10) NOT NULL CHECK (asset_type IN ('classic', 'sac', 'soroban')),
    asset_code       VARCHAR(12),
    issuer_address   VARCHAR(56),
    contract_id      VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    name             VARCHAR(100),
    total_supply     NUMERIC(28, 7),
    holder_count     INT DEFAULT 0,
    metadata         JSONB,
    UNIQUE (asset_code, issuer_address),
    UNIQUE (contract_id)
);
```

### 6.9 Accounts

```sql
CREATE TABLE accounts (
    account_id        VARCHAR(56) PRIMARY KEY,
    first_seen_ledger BIGINT REFERENCES ledgers(sequence),
    last_seen_ledger  BIGINT REFERENCES ledgers(sequence),
    sequence_number   BIGINT,
    balances          JSONB NOT NULL DEFAULT '[]'::jsonb,
    home_domain       VARCHAR(255),
    INDEX idx_last_seen (last_seen_ledger DESC)
);
```

### 6.10 NFTs

```sql
CREATE TABLE nfts (
    id                BIGSERIAL PRIMARY KEY,
    contract_id       VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    token_id          VARCHAR(128) NOT NULL,
    collection_name   VARCHAR(100),
    owner_account     VARCHAR(56),
    name              VARCHAR(100),
    media_url         TEXT,
    metadata          JSONB,
    minted_at_ledger  BIGINT REFERENCES ledgers(sequence),
    last_seen_ledger  BIGINT REFERENCES ledgers(sequence),
    UNIQUE (contract_id, token_id),
    INDEX idx_contract (contract_id),
    INDEX idx_owner (owner_account)
);
```

### 6.11 Liquidity Pools

```sql
CREATE TABLE liquidity_pools (
    pool_id             VARCHAR(64) PRIMARY KEY,
    asset_a             JSONB NOT NULL,
    asset_b             JSONB NOT NULL,
    fee_bps             INT,
    reserves            JSONB NOT NULL,
    total_shares        NUMERIC(28, 7),
    tvl                 NUMERIC(28, 7),
    created_at_ledger   BIGINT REFERENCES ledgers(sequence),
    last_updated_ledger BIGINT REFERENCES ledgers(sequence),
    INDEX idx_last_updated (last_updated_ledger DESC)
);
```

### 6.12 Liquidity Pool Snapshots

```sql
CREATE TABLE liquidity_pool_snapshots (
    id               BIGSERIAL PRIMARY KEY,
    pool_id          VARCHAR(64) REFERENCES liquidity_pools(pool_id) ON DELETE CASCADE,
    ledger_sequence  BIGINT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    reserves         JSONB NOT NULL,
    total_shares     NUMERIC(28, 7),
    tvl              NUMERIC(28, 7),
    volume           NUMERIC(28, 7),
    fee_revenue      NUMERIC(28, 7),
    INDEX idx_pool_time (pool_id, created_at DESC)
) PARTITION BY RANGE (created_at);
```

### 6.13 Partitioning and Retention

Tables `soroban_invocations`, `soroban_events`, and `liquidity_pool_snapshots` are
partitioned by month using native PostgreSQL range partitioning. The `operations` table is
partitioned separately by `transaction_id` in the current schema. A cleanup Lambda
(EventBridge daily) creates partitions 2 months ahead and drops partitions older than the
retention window if storage constraints require it. Ledger and transaction tables are not
partitioned and are kept indefinitely.

---

## 7. Estimates

### 7.1 Effort Breakdown by Project Part

#### A. Design — 35–40 days (runs before / in parallel with Phase 1)

#### B. AWS Architecture + Galexie Infrastructure

| Task                                                      | Days   |
| --------------------------------------------------------- | ------ |
| VPC, subnets, security groups, IAM roles (CDK)            | 4      |
| ECS Fargate cluster + Galexie task definition + S3 bucket | 5      |
| Galexie configuration and testnet validation              | 3      |
| Lambda + API Gateway setup (NestJS deployment pipeline)   | 4      |
| CloudFront CDN + Route 53 + TLS                           | 1      |
| Secrets Manager, CloudWatch dashboards, X-Ray             | 2      |
| Historical backfill ECS task + monitoring                 | 5      |
| CI/CD pipeline (GitHub Actions → CDK)                     | 4      |
| Staging + production environment parity                   | 4      |
| **Subtotal**                                              | **32** |

#### C. Data Ingestion Pipeline

| Task                                                                                      | Days   |
| ----------------------------------------------------------------------------------------- | ------ |
| Ledger Processor Lambda — XDR parse + DB write (ledgers, txs, ops, accounts, NFTs, pools) | 6      |
| Ledger Processor — Soroban invocations + CAP-67 events extraction                         | 5      |
| Ledger Processor — contract deployments + token/NFT/pool detection                        | 4      |
| Event Interpreter Lambda — human-readable summaries                                       | 5      |
| Backfill validation — gap detection, idempotency checks                                   | 3      |
| Ingestion lag monitoring + alerting                                                       | 2      |
| **Subtotal**                                                                              | **25** |

#### D. Core API Endpoints (NestJS)

| Task                                                             | Days   |
| ---------------------------------------------------------------- | ------ |
| NestJS project scaffolding, module structure, Drizzle ORM setup  | 3      |
| Network stats endpoint                                           | 1      |
| Transactions endpoints (list + detail + operation tree)          | 9      |
| Ledgers endpoints (list + detail)                                | 3      |
| Accounts endpoints (detail + transactions/history)               | 4      |
| Tokens endpoints (list + detail + transactions)                  | 5      |
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
| Tokens page (list, filters)                                          | 2      |
| Token detail page (summary + transactions)                           | 2      |
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
| Unit tests — API endpoints (NestJS)                         | 8      |
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

| Service                    | Configuration                                        | Monthly Cost    |
| -------------------------- | ---------------------------------------------------- | --------------- |
| ECS Fargate — Galexie      | 1 vCPU / 2 GB RAM, continuous                        | ~$36            |
| RDS PostgreSQL             | db.r6g.large, Single-AZ                              | ~$175           |
| RDS Storage                | 1 TB gp3 (full chain data from 2023)                 | ~$115           |
| API Gateway                | 1M requests + 0.5 GB cache                           | ~$4             |
| Lambda — API handlers      | 800K invocations, 512 MB ARM                         | ~$5             |
| Lambda — Ingestion workers | ~500K invocations (Ledger Processor)                 | ~$10            |
| CloudFront                 | 10 GB transfer                                       | ~$5             |
| S3                         | Ledger XDR files (no auto-deletion, grows over time) | ~$5+            |
| NAT Gateway                | 1x, ~100 GB data                                     | ~$40            |
| CloudWatch + X-Ray         | Logs, metrics, tracing                               | ~$20            |
| Secrets Manager + Route 53 | Credentials + DNS                                    | ~$10            |
| **Total**                  |                                                      | **~$425/month** |

#### Scaling Path to High Traffic (10M requests/month)

| Change                                           | Trigger           | Added Cost      |
| ------------------------------------------------ | ----------------- | --------------- |
| Add Lambda provisioned concurrency (5 instances) | >2 req/s avg      | +$75            |
| Add RDS read replica (db.r6g.large)              | Primary CPU >60%  | +$175           |
| Enable RDS Multi-AZ                              | SLA >99.9% needed | +$175           |
| Expand VPC to Multi-AZ                           | With Multi-AZ RDS | +$35 (NAT)      |
| API Gateway + Lambda growth                      | Proportional      | +$30            |
| CloudFront / data transfer growth                | Proportional      | +$20            |
| **Estimated total at 10M requests/month**        |                   | **~$935/month** |

### 7.4 Three-Milestone Delivery Plan

#### Deliverable 1 — Indexing Pipeline & Core Infrastructure

Galexie ECS Fargate task running on mainnet, writing `LedgerCloseMeta` XDR files to S3
every ~5–6 seconds. Lambda Ledger Processor triggered per file, parsing and writing
ledgers, transactions, operations, accounts, Soroban invocations, and CAP-67 events to a
dedicated RDS PostgreSQL database. Historical backfill from Soroban mainnet activation ledger
(late 2023). NestJS API scaffolding with core modules. OpenAPI specification. AWS CDK
infrastructure-as-code. CI/CD pipeline. CloudWatch dashboards and ingestion lag alarms.

**Acceptance criteria:**

1. S3 bucket contains consecutive `LedgerCloseMeta` files with timestamps matching
   mainnet ledger close times
2. RDS `ledgers` table contains all ledgers from backfill start through current tip with
   no gaps
3. RDS `soroban_events` table contains CAP-67 events for known Soroswap/Aquarius/Phoenix
   transactions (spot-checked by transaction hashes)
4. `cdk deploy` from a clean AWS account produces the full working stack with no manual
   steps
5. CloudWatch dashboard accessible; Galexie lag alarm fires correctly in staging

**Budget: $26,240 (20% of total)**

---

#### Deliverable 2 — Complete API + Frontend

All REST API endpoints live and serving mainnet data: transactions (list + detail),
ledgers (list + detail), accounts (detail + history), contracts (detail + invocations +
events), tokens, NFTs, liquidity pools, search (exact match + prefix). Human-readable
event interpretation for known patterns (swaps, transfers, mints) on transaction detail
pages. React SPA deployed via CloudFront with all pages. Rate limiting and response
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
parsing correctness, API endpoint responses, and event interpretation logic. Load test
results documented (1M baseline, 10M stress). Security audit checklist (OWASP Top 10,
IAM least-privilege, no public RDS endpoint). Monitoring dashboards and alerting active
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
   ingress, RDS has no public endpoint, production RDS backups/PITR/deletion protection
   enabled, RDS and S3 encrypted with KMS-backed keys, all secrets in Secrets Manager, all
   API inputs validated
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
- **Event interpretation coverage** — human-readable summaries rely on heuristics per
  known protocol (Soroswap, Aquarius, Phoenix). Unknown contracts will show raw decoded
  event data. Coverage expands incrementally as new protocols are identified.
