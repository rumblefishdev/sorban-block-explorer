# Stellar Block Explorer - Database Schema Overview

> This document expands the database schema portion of
> [`technical-design-general-overview.md`](../technical-design-general-overview.md).
> It preserves the same schema scope and storage assumptions, but specifies the model in
> more detail so it can later serve as input for implementation task planning.

---

## Table of Contents

1. [Purpose and Scope](#1-purpose-and-scope)
2. [Ownership and Design Goals](#2-ownership-and-design-goals)
3. [Schema Shape Overview](#3-schema-shape-overview)
4. [Table Design](#4-table-design)
5. [Relationships and Data Flow](#5-relationships-and-data-flow)
6. [Indexing, Partitioning, and Retention](#6-indexing-partitioning-and-retention)
7. [Read and Write Patterns](#7-read-and-write-patterns)
8. [Evolution Rules and Delivery Notes](#8-evolution-rules-and-delivery-notes)

---

## 1. Purpose and Scope

The database schema is the persistent storage model of the block explorer. Its role is to
store all indexed chain data needed by the ingestion pipeline, backend API, and explorer UI
without depending on any external explorer database.

This document covers the target design of the PostgreSQL schema only. It does not redefine
frontend behavior, backend transport concerns, or infrastructure provisioning except where
those influence schema decisions.

This document describes the intended production schema model. It is not a description of
current implementation state in the repository, which is still skeletal.

If any statement in this file conflicts with
[`technical-design-general-overview.md`](../technical-design-general-overview.md), the
general overview document takes precedence. This file is a database-schema-focused
refinement of that source, not an independent redesign.

## 2. Ownership and Design Goals

The block explorer owns its full PostgreSQL schema. All chain data is stored here; there is
no dependency on an external database.

The schema should satisfy four goals at the same time:

- support deterministic ingestion from `LedgerCloseMeta`-derived data
- support fast read patterns for explorer APIs and list/detail pages
- retain raw protocol payloads where advanced inspection requires them
- separate normalized explorer reads from low-level ledger extraction concerns

### 2.1 Schema Principles

The current design implies the following principles:

- `ledgers` and `transactions` are the backbone of the explorer timeline
- Soroban-specific entities are modeled explicitly instead of being hidden inside generic
  JSON blobs only
- JSONB is used where protocol payloads are naturally nested or variably shaped
- relational links are still used for the main explorer graph so joins remain predictable
- partitioning is used selectively for high-volume, time-oriented tables

### 2.2 What the Schema Is Not

The schema is not intended to be:

- a generic mirror of every Stellar ledger entry type
- a third-party-compatible Horizon clone
- a user/account-auth database for end-user sessions or registration
- a storage layer that requires runtime access to external APIs for core reads

## 3. Schema Shape Overview

The current schema shape is centered around a small set of core explorer entities:

- `ledgers` as the ledger-close timeline
- `transactions` as the primary explorer activity entity
- `operations_appearances` as a transaction-scoped appearance index for classic and mixed transaction inspection (per-op detail recovered from XDR on demand, task 0163)
- `soroban_contracts`, `soroban_invocations`, and `soroban_events` as the Soroban-native
  contract activity model
- `assets`, `accounts`, `nfts`, and `liquidity_pools` as derived, query-oriented explorer
  entities built on indexed state

High-level relationship sketch:

```text
ledgers
  └─ transactions
       ├─ operations_appearances
       ├─ soroban_invocations
       └─ soroban_events

soroban_contracts
  ├─ soroban_invocations
  ├─ soroban_events
  ├─ assets
  └─ nfts

liquidity_pools
  └─ liquidity_pool_snapshots

accounts
  └─ referenced by transaction and contract/account identity fields
```

This is not a full ERD. It is the intended logical shape that the API and ingestion
pipeline depend on.

## 4. Table Design

### 4.1 Ledgers

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

Purpose:

- represent the canonical ledger timeline
- anchor transaction grouping and ledger-detail pages
- support recent-ledger browsing and monotonic sequence navigation

Design notes:

- `sequence` is the natural stable primary key for ledger navigation
- `hash` is unique but not the primary explorer lookup key in current routes
- `closed_at` supports recent-history ordering and freshness comparisons

### 4.2 Transactions

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

Purpose:

- act as the primary explorer entity for activity browsing and detail views
- preserve raw XDR needed for advanced/debugging output
- support transaction-detail tree rendering without reparsing result meta for every request
- carry the main transaction summary fields used across routes

Design notes:

- `id` provides an internal surrogate key for child tables
- `hash` is the main public lookup key for transaction detail routes
- `ledger_sequence` links each transaction back to the ledger timeline
- `result_meta_xdr` preserves raw transaction meta for advanced decode/debug recovery paths
- `operation_tree` stores decoded invocation hierarchy for detail renderers
- `parse_error` allows partial retention even when full decode fails

### 4.3 Operations Appearances

```sql
CREATE TABLE operations_appearances (
    id                BIGSERIAL    NOT NULL,
    transaction_id    BIGINT       NOT NULL,
    type              SMALLINT     NOT NULL,  -- ADR 0031 OperationType
    source_id         BIGINT       REFERENCES accounts(id),
    destination_id    BIGINT       REFERENCES accounts(id),
    contract_id       BIGINT       REFERENCES soroban_contracts(id),
    asset_code        VARCHAR(12),
    asset_issuer_id   BIGINT       REFERENCES accounts(id),
    pool_id           BYTEA,
    amount            BIGINT       NOT NULL,  -- count of collapsed identical-identity ops
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

Purpose (task 0163):

- serve as an **appearance index** (pattern from `soroban_events_appearances` /
  `soroban_invocations_appearances`, ADR 0033/0034) — one row per distinct
  operation identity in a transaction; `amount` counts how many operations of
  that shape were folded into the row
- let filter/search endpoints answer _whether_ an operation of a given shape
  occurred against a given account / contract / asset / pool, without reading
  XDR
- delegate per-op detail (transfer amount, application order, memo, claimants,
  function args, predicates, …) to the XDR archive — the API re-materialises
  it on demand via `xdr_parser::extract_operations`

Design notes:

- identity columns (`type` … `pool_id`) match the `uq_ops_app_identity`
  natural key; `NULLS NOT DISTINCT` (PG 15+) makes NULL-heavy shapes (e.g.
  `CREATE_CLAIMABLE_BALANCE`, source inherited from tx) idempotent on replay
  via `ON CONFLICT DO NOTHING`
- no `transfer_amount` / `application_order` column — both are recoverable
  from XDR; keeping them here duplicated write-only data with no reader
- `ON DELETE CASCADE` via the composite FK mirrors the rest of the schema
- partitioned by `created_at` (monthly) per ADR 0027 — uniform with
  `transactions`, `transaction_participants`, `soroban_events_appearances`,
  `soroban_invocations_appearances`, `liquidity_pool_snapshots`

### 4.4 Soroban Contracts

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

Purpose:

- represent deployed Soroban contracts as first-class explorer entities
- support contract-detail pages, interface display, and search
- classify contracts into explorer-relevant categories

Design notes:

- `contract_id` is the public stable identifier
- `metadata` is flexible because contract metadata quality and shape may vary; current
  design also allows it to hold optional extracted interface signatures for the contract
- `search_vector` makes contract-name and metadata-driven search efficient

### 4.5 Soroban Invocations

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

Purpose:

- store decoded contract-call activity in a queryable form
- support contract invocation history and function-centric views
- avoid reparsing invocation payloads for every backend request

Design notes:

- `function_args` and `return_value` are JSONB because decoded `ScVal` shapes vary
- `created_at` supports partitioning and recent-history access patterns
- `ledger_sequence` keeps ledger ordering explicit even where timestamps are primary for reads

### 4.6 Soroban Events (CAP-67)

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

Purpose:

- persist decoded Soroban event streams in structured form
- support transaction-detail advanced sections and contract event tabs
- support pattern matching and downstream interpretation jobs

Design notes:

- `topics` and `data` are JSONB because decoded event payloads are structurally variable
- `event_type` distinguishes contract/system/diagnostic event classes
- `idx_topics` exists to support query patterns based on event signatures or topic structure

### 4.7 Assets

```sql
CREATE TABLE assets (
    id          SERIAL   PRIMARY KEY,
    asset_type  SMALLINT NOT NULL,  -- TokenAssetType: 0=native, 1=classic_credit, 2=sac, 3=soroban
    asset_code  VARCHAR(12),
    issuer_id   BIGINT   REFERENCES accounts(id),
    contract_id BIGINT   REFERENCES soroban_contracts(id),
    name        VARCHAR(256),
    total_supply NUMERIC(28,7),
    holder_count INTEGER,
    description TEXT,
    icon_url    VARCHAR(1024),
    home_page   VARCHAR(256),
    CONSTRAINT ck_assets_asset_type_range CHECK (asset_type BETWEEN 0 AND 15),
    CONSTRAINT ck_assets_identity CHECK (
        (asset_type = 0 AND asset_code IS NULL     AND issuer_id IS NULL     AND contract_id IS NULL)
     OR (asset_type = 1 AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
     OR (asset_type = 2 AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NOT NULL)
     OR (asset_type = 3 AND issuer_id IS NULL      AND contract_id IS NOT NULL)
    )
);
-- partial unique indexes enforce one row per logical asset:
CREATE UNIQUE INDEX uidx_assets_native        ON assets ((asset_type)) WHERE asset_type = 0;
CREATE UNIQUE INDEX uidx_assets_classic_asset ON assets (asset_code, issuer_id) WHERE asset_type IN (1, 2);
CREATE UNIQUE INDEX uidx_assets_soroban       ON assets (contract_id)           WHERE asset_type IN (2, 3);
```

Purpose:

- unify all Stellar asset classes (native XLM, classic credit assets, SACs, Soroban-native
  tokens) in one explorer-facing model — renamed from `tokens` in task 0154 to align with
  the official Stellar taxonomy
- support asset lists and detail pages without splitting the UI into separate products
- preserve the identity differences between asset classes via `ck_assets_identity`

Design notes:

- `asset_type` is a `SMALLINT` enum (`TokenAssetType`), not a `VARCHAR` CHECK — label helper
  `token_asset_type_name(ty)` maps discriminants to readable strings
- native XLM is uniquely identified by `asset_type = 0`; classic credit and SAC assets by
  `asset_code + issuer_id`; SAC and Soroban assets by `contract_id`
- `soroban_contracts.contract_type = 'token'` classifies a contract's SEP-41 role and is
  intentionally distinct from this table's name — the two coexist without ambiguity

### 4.8 Accounts

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

Purpose:

- support the currently documented explorer account scope
- expose account summary and balances without recomputing everything at request time
- anchor the account-detail route and account-related searches

Design notes:

- current account scope is intentionally limited to summary, balances, and recent transactions
- the schema persists the subset of account state required by the current product scope
- richer account-state persistence should be added explicitly only if the source document
  expands account functionality

### 4.9 NFTs

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

Purpose:

- model explorer-visible NFT identities and current ownership state
- support NFT list/detail views without reconstructing each token on demand
- keep media and metadata available when known NFT contract patterns expose them

Design notes:

- `token_id` uniqueness is scoped by `contract_id`
- `metadata` and `media_url` remain optional because NFT contract conventions vary heavily
- NFT transfer history is primarily derived from stored events and linked transactions,
  not a separate canonical NFT-ledger table in the current baseline schema

### 4.10 Liquidity Pools

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

Purpose:

- model current pool state for explorer detail and list views
- support pool search and summary reads without recomputing from raw ledger entries on
  every request
- keep current reserves and total shares accessible for pool-overview endpoints

Design notes:

- asset pair and reserve payloads are JSONB because pool assets may span classic and
  Soroban-native identities
- pool transaction history is derived from transactions, operations_appearances, and Soroban events
  rather than a dedicated canonical pool-transactions table in the current baseline schema

### 4.11 Liquidity Pool Snapshots

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

Purpose:

- persist time-series pool state for chart endpoints and recent-trend analysis
- decouple pool-chart reads from live recomputation over raw transaction history
- align pool analytics storage with the explorer's partitioned time-series approach

Design notes:

- snapshot rows are append-only and should be written in ledger order
- `created_at` drives interval queries and monthly partition management
- metrics such as `volume` and `fee_revenue` are explorer-level derived measures, not new
  chain primitives

## 5. Relationships and Data Flow

### 5.1 Ingestion Flow into the Schema

The schema is populated by the Galexie-based ingestion pipeline described in the main
technical design.

At a high level:

- one ledger close produces one ledger record
- each ledger produces many transaction records
- each transaction may produce operation appearances, contract invocation appearances, and event appearances (detailed payloads re-materialised from XDR)
- derived explorer entities such as assets, accounts, NFTs, and liquidity pools are updated
  from extracted state and known event patterns
- liquidity pool snapshots are appended as time-series records for chart-oriented reads

### 5.2 Child-Entity Lifecycle

The schema models a parent-child structure where appropriate:

- deleting a transaction should clean up dependent operation, invocation, and event appearance rows
- contract-linked entities remain queryable through `contract_id` relationships

### 5.3 Public Lookup Keys vs Internal Keys

The model uses a mix of public identifiers and internal surrogate keys:

- public explorer lookups use keys like `hash`, `sequence`, `contract_id`, and `account_id`
- internal joins often rely on surrogate IDs such as `transactions.id` and `soroban_events.id`

This is appropriate because the public explorer model and internal relational model serve
slightly different purposes.

## 6. Indexing, Partitioning, and Retention

### 6.1 Indexing Strategy

The current schema uses indexes for three main reasons:

- fast public lookup by canonical identifier
- efficient recent-history access by time or ledger order
- selective JSONB / full-text access for variable-shaped Soroban and metadata payloads

Notable patterns already present in the source design:

- unique identifier indexes on `ledgers.hash` and `transactions.hash`
- time-oriented indexes such as `idx_closed_at` and `idx_last_seen`
- GIN indexes for full-text fields such as `soroban_contracts.search_vector`
  (legacy `operations.details` / `soroban_events.topics` JSONB GIN indexes
  were dropped when those payloads moved to the XDR archive — ADR 0018, task 0163)

### 6.2 Partitioning Strategy

Per ADR 0027, all high-volume child tables are partitioned by month on
`created_at`; lightweight anchor/registry tables stay unpartitioned:

- **Partitioned (`RANGE (created_at)` monthly):** `transactions`,
  `operations_appearances`, `transaction_participants`,
  `soroban_invocations_appearances`, `soroban_events_appearances`,
  `nft_ownership`, `liquidity_pool_snapshots`
- **Unpartitioned:** `ledgers`, `transaction_hash_index`, `accounts`,
  `soroban_contracts`, `assets`, `nfts`, `liquidity_pools`
- Partitioning exists to keep retention, maintenance, and time-sliced reads
  practical on the high-write tables

### 6.3 Retention Model

The current retention statement is conservative:

- ledger and transaction history are kept indefinitely
- partitioned time-series tables may be pruned only if storage constraints require it
- partitions are created ahead of time and dropped operationally, not ad hoc by application code

This supports public-explorer expectations better than aggressive retention on core history.

## 7. Read and Write Patterns

### 7.1 Write Patterns

The schema is write-heavy during ingestion and read-heavy during explorer use.

Write-side characteristics:

- append-oriented ledger and transaction ingestion committed in per-ledger database
  transactions
- batch insertion of child rows per processed ledger file with replay-safe replacement or
  de-duplication for the same ledger sequence
- derived-state upserts for entities such as `assets`, `accounts`, `nfts`, and
  `liquidity_pools`, guarded by ledger-sequence watermarks so older batches cannot overwrite
  newer state
- append-only writes for `liquidity_pool_snapshots` used by chart endpoints

### 7.2 Read Patterns

The backend and frontend imply predictable read categories:

- recent ledgers and recent transactions lists
- exact lookup by transaction hash, contract ID, account ID, asset identity, NFT identity,
  pool ID, or ledger sequence
- contract-centric timelines for invocations and events
- asset-centric, account-centric, and NFT-centric recent-activity views
- liquidity-pool detail, transaction, and chart reads
- search over metadata and canonical identifiers

The schema should continue to prioritize those explorer patterns over generic analytical use cases.

### 7.3 Raw vs Derived Storage

The design deliberately stores both raw and derived forms where needed:

- raw XDR for advanced inspection (`transactions.envelope_xdr`, `transactions.result_xdr`,
  `transactions.result_meta_xdr`)
- appearance indexes for normal explorer views (`operations_appearances`,
  `soroban_invocations_appearances`, `soroban_events_appearances`) — per-row
  detail is re-materialised from the XDR archive by the API on demand
- time-series derived forms in `liquidity_pool_snapshots`

This is a core architectural choice, not accidental duplication.

## 8. Evolution Rules and Delivery Notes

### 8.1 Schema Evolution Rules

Any future schema change should preserve the same general discipline:

- add new tables or columns only when tied to a documented explorer or ingestion need
- avoid replacing explicit relational structure with oversized generic JSON blobs
- keep public lookup keys stable where routes or API contracts depend on them
- update the general overview first if the conceptual schema changes materially

### 8.2 Current Workspace State

The repository currently provides architectural documentation and bounded-context package
structure, but not the final migration or runtime persistence implementation yet.

That is expected. This document should serve as the detailed schema reference for future
implementation planning, while
[`technical-design-general-overview.md`](../technical-design-general-overview.md) remains
the primary source of truth.
