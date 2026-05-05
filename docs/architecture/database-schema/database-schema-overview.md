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

This document describes the **current production schema** as of the migrations in
`crates/db/migrations/` (post-ADR 0036 rename `tokens → assets`); every DDL block
in §4 matches the live migration state. The narrative
[`technical-design-general-overview.md`](../technical-design-general-overview.md)
takes precedence for cross-component behavior, but where its §6 data model and this
file disagree on schema specifics, this file is authoritative — it is kept in sync
with the migrations per
[ADR 0032](../../../lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md).

## 2. Ownership and Design Goals

The block explorer owns its full PostgreSQL schema. All chain data is stored here; there is
no dependency on an external database.

The schema should satisfy four goals at the same time:

- support deterministic ingestion from `LedgerCloseMeta`-derived data
- support fast read patterns for explorer APIs and list/detail pages
- carry typed summary columns for everything served by list endpoints; defer raw
  protocol payloads to the public Stellar ledger archive, which is re-fetched and
  parsed at request time for the heavy-field endpoints (E3, E14) per
  [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)
- separate normalized explorer reads from low-level ledger extraction concerns

### 2.1 Schema Principles

The current design implies the following principles:

- `ledgers` and `transactions` are the backbone of the explorer timeline
- Soroban-specific entities are modeled explicitly as first-class tables
  (`soroban_contracts`, `wasm_interface_metadata`, appearance indexes) rather than
  being hidden inside generic JSON blobs
- typed columns are preferred over JSONB for anything that participates in a
  closed domain (enums as `SMALLINT` per ADR 0031, hashes as `BYTEA(32)` per
  ADR 0024, balances as `NUMERIC(28,7)`); JSONB is reserved for genuinely open
  metadata shapes (`soroban_contracts.metadata`, `wasm_interface_metadata.metadata`,
  `nfts.metadata`)
- relational links are always surrogate `BIGINT` FKs to `accounts.id` /
  `soroban_contracts.id` (ADRs 0026 / 0030); StrKeys stay as public lookup keys but
  are not joined on internally
- partitioning is used selectively for high-volume, time-oriented tables; monthly
  `RANGE (created_at)` per ADR 0027

### 2.2 What the Schema Is Not

The schema is not intended to be:

- a generic mirror of every Stellar ledger entry type
- a third-party-compatible Horizon clone
- a user/account-auth database for end-user sessions or registration
- self-sufficient for heavy-field inspection: the public Stellar ledger archive is
  a required read-path dependency for full transaction envelopes, raw result meta,
  and full invocation/event decoding (see ADR 0029); list endpoints and all
  partition-pruned reads remain DB-local

## 3. Schema Shape Overview

The current schema is centered around a small set of core explorer entities plus a
handful of registry and history tables. Table names below are the physical names used
by the current migrations (`crates/db/migrations/0001_*` through `0007_*`).

Backbone timeline:

- `ledgers` — ledger-close timeline (anchor)
- `transactions` — primary explorer activity entity (partitioned by `created_at`)
- `transaction_hash_index` — unpartitioned hash-to-ledger lookup for direct detail routes
- `operations_appearances` — transaction-scoped appearance index for classic and
  mixed transaction inspection (partitioned; per-op detail recovered from XDR on
  demand per task 0163)
- `transaction_participants` — derived participant links for account-history reads (partitioned)

Soroban activity model (per ADRs 0033/0034 these are pure appearance indexes — parsed
contract-event and invocation-tree payloads are fetched at read time from the public
Stellar archive, not stored in the DB):

- `soroban_contracts` — deployed contracts (`BIGSERIAL id` + `VARCHAR(56)` natural `contract_id`)
- `wasm_interface_metadata` — WASM ABI keyed by `wasm_hash`
- `soroban_events_appearances` — contract-event appearance index (partitioned)
- `soroban_invocations_appearances` — contract-invocation appearance index (partitioned)

Derived explorer entities:

- `assets` — unified asset registry (native, classic_credit, SAC, Soroban SEP-41);
  renamed from `tokens` in ADR 0036 / task 0154
- `accounts` — account identity hub (`BIGSERIAL id` surrogate + `VARCHAR(56)` natural `account_id`)
- `account_balances_current` — classic trustline current balances (history table dropped per ADR 0035)
- `nfts`, `nft_ownership` — NFT registry plus partitioned ownership history
- `liquidity_pools`, `liquidity_pool_snapshots`, `lp_positions` — classic LP state +
  time-series snapshots + per-account share positions

High-level relationship sketch:

```text
ledgers
  └─ transactions (partitioned)
       ├─ operations_appearances (partitioned)
       ├─ transaction_participants (partitioned)
       ├─ soroban_events_appearances (partitioned)
       └─ soroban_invocations_appearances (partitioned)

soroban_contracts
  ├─ wasm_interface_metadata
  ├─ soroban_events_appearances
  ├─ soroban_invocations_appearances
  ├─ assets
  └─ nfts ─ nft_ownership (partitioned)

liquidity_pools
  ├─ liquidity_pool_snapshots (partitioned)
  └─ lp_positions

accounts
  ├─ account_balances_current
  └─ referenced by FK from every table that carries a source/destination/issuer/
     deployer/owner column
```

This is not a full ERD. It is the intended logical shape that the API and ingestion
pipeline depend on.

### 3.1 Surrogate key discipline (ADR 0026, ADR 0030)

`accounts` and `soroban_contracts` both use a `BIGSERIAL id` surrogate primary key while
retaining their natural `VARCHAR(56)` StrKey as a `UNIQUE` column. Every FK column in
other tables targets the surrogate `id` (`BIGINT`), not the StrKey. API routes that
accept a StrKey resolve it to the surrogate at the request boundary; API responses that
display a StrKey join back to `accounts` / `soroban_contracts` for the display value.
The public API surface is unchanged by this rewrite.

### 3.2 Binary hashes (ADR 0024)

Every 32-byte chain hash is stored as `BYTEA` with `CHECK (octet_length(...) = 32)`:
`ledgers.hash`, `transactions.hash`, `transactions.inner_tx_hash`,
`transaction_hash_index.hash`, `soroban_contracts.wasm_hash`,
`wasm_interface_metadata.wasm_hash`, and the 32-byte `pool_id` on
`liquidity_pools` / `liquidity_pool_snapshots` / `lp_positions` / `operations_appearances`.
The domain layer renders each as lowercase hex on the API; no route changes hex
strings into binary.

### 3.3 Enum columns (ADR 0031)

All closed-domain enum columns are `SMALLINT` backed by a Rust `#[repr(i16)]` enum in
`crates/domain/src/enums/`, with a `CHECK` range constraint and a `<name>_name(ty)` SQL
helper function for psql/BI debugging. Columns: `operations_appearances.type`,
`assets.asset_type`, `account_balances_current.asset_type`,
`nft_ownership.event_type`,
`liquidity_pools.asset_a_type`, `liquidity_pools.asset_b_type`,
`soroban_contracts.contract_type`. Parser code binds integers directly; API serializers
render the canonical string.

## 4. Table Design

### 4.1 Ledgers

```sql
CREATE TABLE ledgers (
    sequence          BIGINT      PRIMARY KEY,
    hash              BYTEA       NOT NULL UNIQUE,            -- 32-byte ledger hash (ADR 0024)
    closed_at         TIMESTAMPTZ NOT NULL,
    protocol_version  INTEGER     NOT NULL,
    transaction_count INTEGER     NOT NULL,
    base_fee          BIGINT      NOT NULL,
    CONSTRAINT ck_ledgers_hash_len CHECK (octet_length(hash) = 32)
);
CREATE INDEX idx_ledgers_closed_at ON ledgers (closed_at DESC);
```

Purpose:

- represent the canonical ledger timeline
- anchor transaction grouping and ledger-detail pages
- support recent-ledger browsing and monotonic sequence navigation

Design notes:

- `sequence` is the natural stable primary key for ledger navigation
- `hash` is stored as `BYTEA(32)` per [ADR 0024](../../../lore/2-adrs/0024_hashes-bytea-binary-storage.md)
  and rendered as lowercase hex at the API boundary; it is unique but not the primary
  explorer lookup key in current routes
- `closed_at` supports recent-history ordering and freshness comparisons

### 4.2 Transactions

```sql
CREATE TABLE transactions (
    id                BIGSERIAL   NOT NULL,
    hash              BYTEA       NOT NULL,                          -- 32-byte tx hash (ADR 0024)
    ledger_sequence   BIGINT      NOT NULL,
    application_order SMALLINT    NOT NULL,
    source_id         BIGINT      NOT NULL REFERENCES accounts(id),  -- ADR 0026 surrogate
    fee_charged       BIGINT      NOT NULL,
    inner_tx_hash     BYTEA,                                         -- fee-bump inner hash
    successful        BOOLEAN     NOT NULL,
    operation_count   SMALLINT    NOT NULL,
    has_soroban       BOOLEAN     NOT NULL DEFAULT false,
    parse_error       BOOLEAN     NOT NULL DEFAULT false,
    created_at        TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),                                    -- composite PK (partition-key rule)
    CONSTRAINT ck_transactions_hash_len       CHECK (octet_length(hash) = 32),
    CONSTRAINT ck_transactions_inner_hash_len CHECK (inner_tx_hash IS NULL OR octet_length(inner_tx_hash) = 32)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_tx_source_created ON transactions (source_id, created_at DESC);
CREATE INDEX idx_tx_ledger         ON transactions (ledger_sequence);
CREATE INDEX idx_tx_has_soroban    ON transactions (created_at DESC) WHERE has_soroban;
CREATE INDEX idx_tx_keyset         ON transactions (created_at DESC, id DESC);  -- task 0132 / ADR 0039 — E02 no-filter keyset
```

Uniqueness on `hash` is enforced by the companion `transaction_hash_index` table
(see §4.3) rather than a direct `UNIQUE` on the partitioned parent — PostgreSQL
only allows partitioned-table uniqueness when the constraint includes the partition
key, which would make a hash-only lookup unnatural.

Purpose:

- act as the primary explorer entity for activity browsing and detail views
- carry the main transaction summary fields used across routes without fetching XDR

Design notes:

- `id` provides an internal `BIGSERIAL` surrogate key referenced by child tables;
  the composite `(id, created_at)` PK lets child tables cascade via the partitioning key
- `hash` is the main public lookup key for transaction detail routes; binary storage
  per [ADR 0024](../../../lore/2-adrs/0024_hashes-bytea-binary-storage.md)
- `source_id` is the `accounts.id` surrogate
  ([ADR 0026](../../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md)); the
  displayed `G...` StrKey is obtained via JOIN back to `accounts.account_id`
- `application_order`, `operation_count`, `has_soroban` support the transaction
  list/detail renderers and Soroban-filtered indexing
- **no raw XDR stored on the row**: envelope / result / result-meta XDR for
  `/transactions/:hash` (E3) is fetched at request time from the public Stellar
  ledger archive and parsed on-demand, per
  [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md);
  `parse_error` flags rows whose summary columns could not be fully populated from
  the ingest-time parse

### 4.3 Transaction Hash Index

```sql
CREATE TABLE transaction_hash_index (
    hash            BYTEA       PRIMARY KEY,         -- 32-byte tx hash (ADR 0024)
    ledger_sequence BIGINT      NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    CONSTRAINT ck_thi_hash_len CHECK (octet_length(hash) = 32)
);
```

Purpose:

- resolve a transaction hash to its `(ledger_sequence, created_at)` coordinates so
  the partitioned `transactions` row can be located with a partition-pruned lookup
- act as the uniqueness enforcement point for transaction hashes (partitioned parent
  cannot carry a hash-only `UNIQUE` constraint)

Design notes:

- small, unpartitioned, hot-cached — every `/transactions/:hash` lookup goes through
  it before touching the partitioned parent

### 4.4 Operations — Appearance Index

Per task 0163, `operations` was collapsed to an appearance index and renamed
to `operations_appearances`. Pattern matches ADRs 0033/0034 for events and
invocations: one row per distinct operation identity per transaction,
`amount BIGINT` counts collapsed duplicates. Per-op detail (envelope decode,
soroban args, memos, claimants, predicates, etc.) is re-materialised from
XDR at read time via the `runtime_enrichment::stellar_archive` extractors.

```sql
CREATE TABLE operations_appearances (
    id                BIGSERIAL    NOT NULL,
    transaction_id    BIGINT       NOT NULL,
    type              SMALLINT     NOT NULL,                               -- ADR 0031 OperationType
    source_id         BIGINT       REFERENCES accounts(id),                -- ADR 0026
    destination_id    BIGINT       REFERENCES accounts(id),                -- ADR 0026
    contract_id       BIGINT       REFERENCES soroban_contracts(id),       -- ADR 0030
    asset_code        VARCHAR(12),
    asset_issuer_id   BIGINT       REFERENCES accounts(id),                -- ADR 0026
    pool_id           BYTEA,                                               -- 32-byte LP hash (ADR 0024)
    amount            BIGINT       NOT NULL,                               -- collapsed-duplicate count
    ledger_sequence   BIGINT       NOT NULL,
    created_at        TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE,
    CONSTRAINT ck_ops_app_pool_id_len CHECK (pool_id IS NULL OR octet_length(pool_id) = 32),
    CONSTRAINT ck_ops_app_type_range  CHECK (type BETWEEN 0 AND 127),      -- ADR 0031 range
    CONSTRAINT ck_ops_app_amount_pos  CHECK (amount > 0),
    CONSTRAINT uq_ops_app_identity    UNIQUE NULLS NOT DISTINCT
        (transaction_id, type, source_id, destination_id,
         contract_id, asset_code, asset_issuer_id, pool_id,
         ledger_sequence, created_at)
) PARTITION BY RANGE (created_at);
```

No explicit `idx_ops_app_tx` — `WHERE transaction_id = X` is served by the
leftmost prefix of `uq_ops_app_identity` (starts with `transaction_id, type, …`).
A dedicated narrower index is reversible via `CREATE INDEX CONCURRENTLY` per
partition if production telemetry shows it's needed.

Purpose:

- index which operation identities appeared in which transaction, with a
  count of how many physical operations collapsed into each identity
- anchor cascade cleanup of transaction children
- preserve the typed summary columns (account/contract/asset/pool surrogates)
  needed for filtered list endpoints without per-request XDR decode

Design notes:

- `type` is `SMALLINT` backed by the Rust `OperationType` enum
  ([ADR 0031](../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md));
  the `op_type_name(ty)` SQL helper renders the canonical string for psql/BI
- every account/contract/issuer reference is a `BIGINT` surrogate FK
  (ADRs 0026 / 0030); `pool_id` is a binary 32-byte pool hash (ADR 0024) with a
  deferred FK attached once `liquidity_pools` exists in migration 0006
- composite `(id, created_at)` PK is required because the partition key must be in
  every unique index on a partitioned table; `created_at` is inherited verbatim from
  the parent transaction so per-partition cascade is well-defined
- `uq_ops_app_identity` uses PG 15+ `NULLS NOT DISTINCT` so NULL-heavy shapes
  (e.g. type-14 `CREATE_CLAIMABLE_BALANCE` with source inherited from tx)
  collapse correctly. Observed compression: 28% overall on backfill sample,
  type-14 collapses from 12 709 operations to 179 rows
- `transfer_amount NUMERIC(28,7)` and `application_order SMALLINT` were
  dropped — no API endpoint reads them, and per-op detail is already
  re-materialised from XDR by `runtime_enrichment::stellar_archive` extractors per
  [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)
- ingest staging aggregates operations at the `HashMap<OpIdentity, i64>`
  level before the bulk INSERT; write layer uses
  `ON CONFLICT ON CONSTRAINT uq_ops_app_identity DO NOTHING` for replay
  idempotency

### 4.5 Transaction Participants

```sql
CREATE TABLE transaction_participants (
    transaction_id BIGINT      NOT NULL,
    account_id     BIGINT      NOT NULL REFERENCES accounts(id),     -- ADR 0026
    created_at     TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (account_id, created_at, transaction_id),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE
) PARTITION BY RANGE (created_at);
CREATE INDEX idx_tp_tx ON transaction_participants (transaction_id);
```

Purpose:

- support account-centric transaction history without table-scanning `transactions`
- dedup per `(account, transaction)` regardless of how many times the account
  appears in a transaction's envelope or result (source, destination, trustor,
  account merge target, etc.)

Design notes:

- per [ADR 0020](../../../lore/2-adrs/0020_tp-drop-role-and-soroban-contracts-index-cut.md)
  the table carries no `role` column — role distinctions live in `operations_appearances`
  (via `source_id`, `destination_id`, `asset_issuer_id`) and `transactions.source_id`,
  which is where the UI already gets them. `transaction_participants` is a pure
  account-feed index
- PK `(account_id, created_at, transaction_id)` is designed for the
  account-feed read pattern (`WHERE account_id = $1 ORDER BY created_at DESC`);
  the secondary `idx_tp_tx` supports the reverse direction
- partitioned on `created_at`, mirrors `transactions` partitions exactly;
  cascade driven by the composite FK back to `transactions`
- `account_id` is the surrogate BIGINT FK per
  [ADR 0026](../../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md)

### 4.6 Soroban Contracts

```sql
CREATE TABLE soroban_contracts (
    id                      BIGSERIAL   PRIMARY KEY,                        -- ADR 0030 surrogate
    contract_id             VARCHAR(56) NOT NULL UNIQUE,                    -- StrKey natural key
    wasm_hash               BYTEA       REFERENCES wasm_interface_metadata(wasm_hash), -- ADR 0024
    wasm_uploaded_at_ledger BIGINT,
    deployer_id             BIGINT      REFERENCES accounts(id),            -- ADR 0026
    deployed_at_ledger      BIGINT,
    contract_type           SMALLINT,                                       -- ADR 0031, nullable
    is_sac                  BOOLEAN     NOT NULL DEFAULT false,
    metadata                JSONB,                                          -- explorer metadata + ABI sigs
    search_vector           TSVECTOR GENERATED ALWAYS AS (
                                to_tsvector('simple', COALESCE(metadata->>'name', '') || ' ' || contract_id)
                            ) STORED,
    CONSTRAINT ck_sc_wasm_hash_len       CHECK (wasm_hash IS NULL OR octet_length(wasm_hash) = 32),
    CONSTRAINT ck_sc_contract_type_range CHECK (contract_type IS NULL OR contract_type BETWEEN 0 AND 15)
);
CREATE INDEX idx_contracts_type   ON soroban_contracts (contract_type);
CREATE INDEX idx_contracts_wasm   ON soroban_contracts (wasm_hash) WHERE wasm_hash IS NOT NULL;
CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
CREATE INDEX idx_contracts_prefix ON soroban_contracts (contract_id text_pattern_ops);
```

Purpose:

- represent deployed Soroban contracts as first-class explorer entities
- support contract-detail pages, interface display, and search
- classify contracts into explorer-relevant roles

Design notes:

- `id` is a `BIGSERIAL` surrogate PK
  ([ADR 0030](../../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md)); `contract_id`
  is kept as the natural StrKey for E22 search, URL routing, and display. Every
  contract FK in other tables (`operations_appearances`, `soroban_events_appearances`,
  `soroban_invocations_appearances`, `assets`, `nfts`) targets `id`
- `wasm_hash` is `BYTEA(32)` (ADR 0024) and FKs into `wasm_interface_metadata`
- `deployer_id` is an `accounts.id` surrogate FK (ADR 0026)
- `contract_type` is `SMALLINT` backed by the Rust `ContractType` enum (ADR 0031);
  nullable because the two-pass upsert in `persist/write.rs` registers bare StrKey
  references before deployment meta is observed — those rows start NULL and get
  filled when the deploy meta lands. The `contract_type_name(ty)` SQL helper renders
  the canonical string
- `metadata` is JSONB (nested ABI function signatures, optional explorer fields)
- `search_vector` combines `metadata->>'name'` and the StrKey, enabling contract
  search on both the friendly name and the canonical identifier

### 4.7 WASM Interface Metadata

```sql
CREATE TABLE wasm_interface_metadata (
    wasm_hash BYTEA PRIMARY KEY,                                       -- 32-byte WASM SHA-256 (ADR 0024)
    metadata  JSONB NOT NULL,                                          -- SEP-48 / interface descriptor
    CONSTRAINT ck_wim_hash_len CHECK (octet_length(wasm_hash) = 32)
);
```

Purpose:

- deduplicate per-WASM ABI metadata across every contract instance that shares
  the same upload (SACs in particular share a single stub WASM)
- back the contract detail page's interface / function-signature tab

Design notes:

- `wasm_hash` is `BYTEA(32)` (ADR 0024); rendered as hex on the API
- `metadata` holds the full decoded SEP-48 shape (ABI functions, enums, spec hash)
- referenced from `soroban_contracts.wasm_hash` (nullable; populated when WASM upload
  is observed — the metadata worker pattern of
  [ADR 0022](../../../lore/2-adrs/0022_schema-correction-and-token-metadata-enrichment.md))

### 4.8 Soroban Events — Appearance Index

```sql
CREATE TABLE soroban_events_appearances (
    contract_id     BIGINT       NOT NULL REFERENCES soroban_contracts(id),  -- ADR 0030
    transaction_id  BIGINT       NOT NULL,
    ledger_sequence BIGINT       NOT NULL,
    amount          BIGINT       NOT NULL,                                  -- non-diagnostic events in trio
    created_at      TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (contract_id, transaction_id, ledger_sequence, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_sea_contract_ledger ON soroban_events_appearances
    (contract_id, ledger_sequence DESC, created_at DESC);
CREATE INDEX idx_sea_transaction     ON soroban_events_appearances
    (transaction_id, created_at DESC);
-- task 0132 / ADR 0039 — E02 Statement B (variant 2)
CREATE INDEX idx_sea_contract_keyset ON soroban_events_appearances
    (contract_id, created_at DESC, transaction_id DESC);
```

Purpose:

- index which contract emitted events in which `(transaction, ledger)` tuple, with a
  count of non-diagnostic events in the tuple
- back the contract detail page's "events" tab and the transaction detail's event list

Design notes:

- this is a pure **appearance index** — the parsed event payload (event type, topics,
  data, per-event index within a tx, transfer triple) is **not** stored in the DB. It
  is fetched at read time from the public Stellar ledger archive and re-expanded on
  demand via `xdr_parser::extract_events`. Formalised by
  [ADR 0033](../../../lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md)
  on top of
  [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)'s
  read-time XDR fetch policy
- `contract_id` is the `BIGINT` surrogate FK per
  [ADR 0030](../../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md)
- partitioned on `created_at` mirroring `transactions`; cascade via composite FK
- diagnostic events are filtered on ingest (they are not counted in `amount` and do
  not produce appearance rows); the detail view re-derives them on demand if needed

### 4.9 Soroban Invocations — Appearance Index

```sql
CREATE TABLE soroban_invocations_appearances (
    contract_id      BIGINT       NOT NULL REFERENCES soroban_contracts(id), -- ADR 0030
    transaction_id   BIGINT       NOT NULL,
    ledger_sequence  BIGINT       NOT NULL,
    caller_id        BIGINT       REFERENCES accounts(id),                   -- ADR 0026
    amount           INTEGER      NOT NULL,                                  -- invocation-tree nodes in trio
    created_at       TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (contract_id, transaction_id, ledger_sequence, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_sia_contract_ledger ON soroban_invocations_appearances
    (contract_id, ledger_sequence DESC);
CREATE INDEX idx_sia_transaction     ON soroban_invocations_appearances
    (transaction_id);
-- task 0132 / ADR 0039 — E02 Statement B (variant 2)
CREATE INDEX idx_sia_contract_keyset ON soroban_invocations_appearances
    (contract_id, created_at DESC, transaction_id DESC);
```

Purpose:

- index which contract was invoked in which `(transaction, ledger)` tuple, with a
  count of invocation-tree nodes in the tuple and the root-level caller account
- back the contract detail page's "invocations" tab and answer E11's
  `unique_callers` stat via `COUNT(DISTINCT caller_id)` without extra JOINs

Design notes:

- like §4.8, this is a pure **appearance index**; per-node detail (function name,
  per-node index, successful flag, function args, return value, depth) lives at read
  time in the public Stellar archive and is re-expanded by
  `xdr_parser::extract_invocations`. Formalised by
  [ADR 0034](../../../lore/2-adrs/0034_soroban-invocations-appearances-read-time-detail.md)
  on top of ADR 0029's read-time XDR fetch policy
- `caller_id` is the root-level caller of the trio. The staging-time
  `is_strkey_account` filter retains G-accounts verbatim and collapses C-contract
  sub-invocation callers to NULL so that "unique **account** callers" is answerable
  without join gymnastics
- `contract_id` is the `BIGINT` surrogate FK per ADR 0030; `caller_id` is the
  `accounts.id` surrogate per ADR 0026
- partitioning and cascade identical to §4.8

### 4.10 Assets

```sql
CREATE TABLE assets (
    id           SERIAL        PRIMARY KEY,
    asset_type   SMALLINT      NOT NULL,   -- TokenAssetType: 0=native, 1=classic_credit, 2=sac, 3=soroban
    asset_code   VARCHAR(12),
    issuer_id    BIGINT        REFERENCES accounts(id),           -- ADR 0026
    contract_id  BIGINT        REFERENCES soroban_contracts(id),  -- ADR 0030
    name         VARCHAR(256),
    total_supply NUMERIC(28,7),                                   -- populated by metadata worker (ADR 0022)
    holder_count INTEGER,                                         -- ditto
    icon_url     VARCHAR(1024),                                   -- list-level thumbnail (ADR 0037 / task 0164)
    CONSTRAINT ck_assets_asset_type_range CHECK (asset_type BETWEEN 0 AND 15),
    -- asset_type = 2 (SAC) admits two shapes — classic-credit wrap carries
    -- (code + issuer + contract); native XLM wrap carries (NULL + NULL +
    -- contract). See ADR 0038.
    CONSTRAINT ck_assets_identity CHECK (
        (asset_type = 0 AND asset_code IS NULL     AND issuer_id IS NULL     AND contract_id IS NULL)
     OR (asset_type = 1 AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
     OR (asset_type = 2 AND contract_id IS NOT NULL AND (
            (asset_code IS NOT NULL AND issuer_id IS NOT NULL)   -- classic-credit SAC
         OR (asset_code IS NULL     AND issuer_id IS NULL)        -- native XLM-SAC (ADR 0038)
        ))
     OR (asset_type = 3 AND issuer_id IS NULL      AND contract_id IS NOT NULL)
    )
);
-- partial unique indexes enforce one row per logical asset:
CREATE UNIQUE INDEX uidx_assets_native        ON assets ((asset_type)) WHERE asset_type = 0;
CREATE UNIQUE INDEX uidx_assets_classic_asset ON assets (asset_code, issuer_id) WHERE asset_type IN (1, 2);
CREATE UNIQUE INDEX uidx_assets_soroban       ON assets (contract_id)           WHERE asset_type IN (2, 3);
CREATE INDEX idx_assets_type      ON assets (asset_type);
CREATE INDEX idx_assets_code_trgm ON assets USING GIN (asset_code gin_trgm_ops);
```

Purpose:

- unify all Stellar asset classes (native XLM, classic credit assets, SACs,
  Soroban-native SEP-41 tokens) in one explorer-facing registry — renamed from
  `tokens` in [ADR 0036](../../../lore/2-adrs/0036_rename-tokens-to-assets.md) /
  task 0154 to align with the official Stellar taxonomy (Stellar "Assets" ≠
  "Tokens" in the Anatomy of an Asset page)
- support asset lists and detail pages without splitting the UI into separate products
- preserve the identity differences between asset classes via `ck_assets_identity`

Design notes:

- `asset_type` is the `SMALLINT` Rust `TokenAssetType` enum per
  [ADR 0031](../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md); label
  helper `token_asset_type_name(ty)` renders strings for psql/BI
- `issuer_id` / `contract_id` are `BIGINT` surrogate FKs (ADRs 0026 / 0030); the
  identity rules in `ck_assets_identity` close the NULL-in-UNIQUE loophole and
  enforce that classic identity fields move together for SAC rows (both set or
  both NULL — see [ADR 0038](../../../lore/2-adrs/0038_loosen-ck-assets-identity-for-native-xlm-sac.md))
- native XLM is uniquely identified by `asset_type = 0`; classic credit and
  classic-credit-wrap SACs by `(asset_code, issuer_id)`; classic-credit-wrap
  SAC, native XLM-SAC, and Soroban-native assets all dedupe by `contract_id`
  via `uidx_assets_soroban`
- the native XLM singleton (`asset_type = 0`, name `"Stellar Lumen"`, all
  identity columns NULL) is bootstrapped by the
  `20260428000000_seed_native_asset_singleton` migration, not by the parser —
  there is no native branch in `detect_assets`. Operator deletion of this row
  breaks the `/assets` listing and any future FK that targets it.
- `icon_url` is the only SEP-1 enrichment field on the DB row — it serves the
  list-page thumbnail (per-row). Asset-detail metadata (`description`,
  `home_page`) lives per-entity in S3 at `s3://<bucket>/assets/{id}.json` per
  [ADR 0037](../../../lore/2-adrs/0037_current-schema-snapshot.md) / task 0164;
  this narrows the original typed-columns plan from
  [ADR 0023](../../../lore/2-adrs/0023_tokens-typed-metadata-columns.md) Part 3
- the SEP-1 / SEP-41 enrichment worker (ADR 0022 pattern) is planned and
  currently unimplemented; when built, it will write `icon_url` to the DB
  and the detail JSON document to S3; not inline with ledger ingest
- `total_supply` and `holder_count` are stock fields also populated post-ingest
- `soroban_contracts.contract_type = 'token'` classifies a contract's SEP-41 role
  and is intentionally distinct from this table's name — the two coexist without
  ambiguity now that the table is `assets`

### 4.11 Accounts

```sql
CREATE TABLE accounts (
    id                BIGSERIAL    PRIMARY KEY,                -- ADR 0026 surrogate
    account_id        VARCHAR(56)  NOT NULL UNIQUE,            -- StrKey G... natural key
    first_seen_ledger BIGINT       NOT NULL,
    last_seen_ledger  BIGINT       NOT NULL,
    sequence_number   BIGINT       NOT NULL,
    home_domain       VARCHAR(256)
);
CREATE INDEX idx_accounts_last_seen ON accounts (last_seen_ledger DESC);
CREATE INDEX idx_accounts_prefix    ON accounts (account_id text_pattern_ops);
```

Purpose:

- anchor the account-detail route, account-related searches, and the surrogate-FK
  resolution path for every table that references an account
- expose account summary fields (last seen, sequence number, home domain) without
  recomputing from raw ledger entries on every request

Design notes:

- `id` is a `BIGSERIAL` surrogate PK per
  [ADR 0026](../../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md); `account_id`
  is kept as the natural `G...` StrKey for display, E22 search, and route lookup
- every `*_id` FK column in the schema that references an account targets `accounts.id`
  (not the StrKey): `transactions.source_id`, `operations_appearances.source_id`,
  `operations_appearances.destination_id`, `operations_appearances.asset_issuer_id`,
  `soroban_contracts.deployer_id`,
  `soroban_invocations_appearances.caller_id`, `assets.issuer_id`, `nfts.current_owner_id`,
  `nft_ownership.owner_id`, `transaction_participants.account_id`,
  `account_balances_current.account_id`,
  `liquidity_pools.asset_a_issuer_id`, `liquidity_pools.asset_b_issuer_id`,
  `lp_positions.account_id`
- account balances live in the dedicated `account_balances_current` table
  (see §4.17), not as JSONB on this row. The previously-planned partitioned
  `account_balance_history` companion was dropped per
  [ADR 0035](../../../lore/2-adrs/0035_drop-account-balance-history.md)

### 4.12 NFTs

```sql
CREATE TABLE nfts (
    id                   SERIAL       PRIMARY KEY,
    contract_id          BIGINT       NOT NULL REFERENCES soroban_contracts(id),  -- ADR 0030
    token_id             VARCHAR(256) NOT NULL,
    collection_name      VARCHAR(256),
    name                 VARCHAR(256),
    media_url            TEXT,
    metadata             JSONB,
    minted_at_ledger     BIGINT,
    current_owner_id     BIGINT       REFERENCES accounts(id),                    -- ADR 0026
    current_owner_ledger BIGINT,
    UNIQUE (contract_id, token_id)
);
CREATE INDEX idx_nfts_collection      ON nfts (collection_name);
CREATE INDEX idx_nfts_collection_trgm ON nfts USING GIN (collection_name gin_trgm_ops);  -- task 0132 / ADR 0039 — E15 ILIKE
CREATE INDEX idx_nfts_owner           ON nfts (current_owner_id);
CREATE INDEX idx_nfts_name_trgm       ON nfts USING GIN (name gin_trgm_ops);
```

Purpose:

- model explorer-visible NFT identities and current ownership state
- support NFT list/detail views without reconstructing each token on demand
- keep media and metadata available when known NFT contract patterns expose them

Design notes:

- `token_id` uniqueness is scoped by `contract_id`; the FK to
  `soroban_contracts.id` is a `BIGINT` surrogate per
  [ADR 0030](../../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md)
- `current_owner_id` is the `accounts.id` surrogate (ADR 0026); the displayed
  `G...` StrKey is obtained via JOIN back to `accounts.account_id`
- `metadata` and `media_url` remain optional because NFT contract conventions vary
  heavily; full transfer history lives in `nft_ownership` (§4.13)

### 4.13 NFT Ownership

```sql
CREATE TABLE nft_ownership (
    nft_id          INTEGER      NOT NULL REFERENCES nfts(id) ON DELETE CASCADE,
    transaction_id  BIGINT       NOT NULL,
    owner_id        BIGINT       REFERENCES accounts(id),              -- ADR 0026
    event_type      SMALLINT     NOT NULL,                             -- ADR 0031 NftEventType
    ledger_sequence BIGINT       NOT NULL,
    event_order     SMALLINT     NOT NULL,
    created_at      TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (nft_id, created_at, ledger_sequence, event_order),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE,
    CONSTRAINT ck_nft_own_event_type_range CHECK (event_type BETWEEN 0 AND 15)
) PARTITION BY RANGE (created_at);
```

Purpose:

- record every mint/transfer/burn event per NFT instance for the NFT detail page's
  history tab
- support owner-centric NFT feeds (account → NFTs currently held + history)

Design notes:

- `event_type` is `SMALLINT` Rust `NftEventType` enum (`0=mint`, `1=transfer`,
  `2=burn`) per [ADR 0031](../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md);
  helper `nft_event_type_name(ty)` for psql/BI
- `owner_id` is the recipient's surrogate account FK (ADR 0026); NULL for burns
- partitioned on `created_at` mirroring `transactions`; cascade via composite FK to
  `transactions` and a direct FK to `nfts`

### 4.14 Liquidity Pools

```sql
CREATE TABLE liquidity_pools (
    pool_id            BYTEA       PRIMARY KEY,                     -- 32-byte pool hash (ADR 0024)
    asset_a_type       SMALLINT    NOT NULL,                        -- ADR 0031 XDR AssetType
    asset_a_code       VARCHAR(12),
    asset_a_issuer_id  BIGINT      REFERENCES accounts(id),         -- ADR 0026
    asset_b_type       SMALLINT    NOT NULL,                        -- ADR 0031 XDR AssetType
    asset_b_code       VARCHAR(12),
    asset_b_issuer_id  BIGINT      REFERENCES accounts(id),         -- ADR 0026
    fee_bps            INTEGER     NOT NULL,
    created_at_ledger  BIGINT      NOT NULL,
    CONSTRAINT ck_lp_pool_id_len        CHECK (octet_length(pool_id) = 32),
    CONSTRAINT ck_lp_asset_a_type_range CHECK (asset_a_type BETWEEN 0 AND 15),
    CONSTRAINT ck_lp_asset_b_type_range CHECK (asset_b_type BETWEEN 0 AND 15)
);
CREATE INDEX idx_pools_asset_a            ON liquidity_pools (asset_a_code, asset_a_issuer_id);
CREATE INDEX idx_pools_asset_b            ON liquidity_pools (asset_b_code, asset_b_issuer_id);
CREATE INDEX idx_pools_created_at_ledger  ON liquidity_pools (created_at_ledger DESC, pool_id DESC);  -- task 0132 / ADR 0039 — E18 keyset
```

Purpose:

- model current classic liquidity pool identity and static fields
- support pool search and detail reads

Design notes:

- `pool_id` is the 32-byte protocol-defined pool hash stored as `BYTEA(32)`
  ([ADR 0024](../../../lore/2-adrs/0024_hashes-bytea-binary-storage.md)); rendered
  as hex at the API boundary
- the asset pair is modeled with **typed columns** (not JSONB): `asset_*_type` is
  the XDR `AssetType` enum (`0=native`, `1=credit_alphanum4`, `2=credit_alphanum12`,
  `3=pool_share`) per [ADR 0031](../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md),
  with `asset_type_name(ty)` SQL helper. Credit-asset pairs carry `asset_*_code` plus
  a `asset_*_issuer_id` `accounts.id` surrogate FK (ADR 0026)
- current reserves and total shares are **not** persisted on the parent row; the most
  recent `liquidity_pool_snapshots` row is the authoritative current-state source
  (pool transaction history itself is derived from `operations_appearances` + `soroban_events_appearances`)
- **Sentinel placeholder rows** ([ADR 0041](../../../lore/2-adrs/0041_lp-positions-orphan-handling-state-filter-and-sentinel-pool.md)):
  during partial / mid-stream backfills, an `lp_positions` row may reference a pool
  whose `LedgerEntry` is not in the current ledger and not previously persisted (the
  pool was created in a pre-window ledger and untouched in the current one). To satisfy
  the FK without losing the position, the persist layer emits a placeholder pool row
  with marker convention **`created_at_ledger = 0`** (no real Stellar pool can carry
  this value — pubnet genesis seq is 1) and minimum-data sentinel fields
  (`asset_a_type=0, asset_a_code=NULL, asset_a_issuer_id=NULL`,
  `asset_b_type=0, asset_b_code=NULL, asset_b_issuer_id=NULL`, `fee_bps=0`). Sentinels
  self-heal: the next time the pool surfaces as `created/updated/restored/state` in
  any subsequent ledger, the 13a UPSERT replaces every dimension field with real
  data. Detection: `WHERE created_at_ledger = 0`. Audit-harness invariant
  `15_liquidity_pools.sql:I6` reports the count as a partial-backfill thermometer.

### 4.15 Liquidity Pool Snapshots

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
    PRIMARY KEY (id, created_at),
    CONSTRAINT ck_lps_pool_id_len CHECK (octet_length(pool_id) = 32)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_lps_pool ON liquidity_pool_snapshots (pool_id, created_at DESC);
CREATE INDEX idx_lps_tvl  ON liquidity_pool_snapshots (tvl DESC) WHERE tvl IS NOT NULL;
```

Purpose:

- persist time-series pool state for chart endpoints and recent-trend analysis
- decouple pool-chart reads from live recomputation over raw transaction history

Design notes:

- snapshot rows are append-only and written in ledger order
- composite `(id, created_at)` PK is required by the partitioning key rule;
  `pool_id` is `BYTEA(32)` (ADR 0024) with the deferred FK back to `liquidity_pools`
- reserves are typed `NUMERIC(28,7)` columns (not JSONB), uniform with the rest of
  the schema's balance / amount handling; `volume` and `fee_revenue` are
  explorer-level derived measures
- `created_at` drives interval queries and monthly partition management

### 4.16 LP Positions

```sql
CREATE TABLE lp_positions (
    pool_id              BYTEA         NOT NULL REFERENCES liquidity_pools(pool_id), -- ADR 0024
    account_id           BIGINT        NOT NULL REFERENCES accounts(id),             -- ADR 0026
    shares               NUMERIC(28,7) NOT NULL,
    first_deposit_ledger BIGINT        NOT NULL,
    last_updated_ledger  BIGINT        NOT NULL,
    PRIMARY KEY (pool_id, account_id),
    CONSTRAINT ck_lpp_pool_id_len CHECK (octet_length(pool_id) = 32)
);
CREATE INDEX idx_lpp_shares ON lp_positions (pool_id, shares DESC) WHERE shares > 0;
```

Purpose:

- track per-account current share balance in each classic liquidity pool for
  account-centric LP reads and pool-participant reads
- back per-pool TVL-by-participant rankings

Design notes:

- unpartitioned current-state table — partial index on `shares > 0` for hot
  listings; closed positions retain a zero-shares row for history lookup
- `pool_id` is `BYTEA(32)` (ADR 0024); `account_id` is the surrogate FK (ADR 0026)

### 4.17 Account Balances (Current)

```sql
CREATE TABLE account_balances_current (
    account_id          BIGINT        NOT NULL REFERENCES accounts(id),     -- ADR 0026
    asset_type          SMALLINT      NOT NULL,                             -- ADR 0031 XDR AssetType
    asset_code          VARCHAR(12),
    issuer_id           BIGINT        REFERENCES accounts(id),              -- ADR 0026
    balance             NUMERIC(28,7) NOT NULL,
    last_updated_ledger BIGINT        NOT NULL,
    CONSTRAINT ck_abc_asset_type_range CHECK (asset_type BETWEEN 0 AND 15),
    CONSTRAINT ck_abc_native
        CHECK ((asset_type =  0 AND asset_code IS NULL     AND issuer_id IS NULL)
            OR (asset_type <> 0 AND asset_code IS NOT NULL AND issuer_id IS NOT NULL))
);
CREATE UNIQUE INDEX uidx_abc_native ON account_balances_current (account_id)
    WHERE asset_type = 0;
CREATE UNIQUE INDEX uidx_abc_credit ON account_balances_current (account_id, asset_code, issuer_id)
    WHERE asset_type <> 0;
CREATE INDEX idx_abc_asset ON account_balances_current (asset_code, issuer_id)
    WHERE asset_code IS NOT NULL;
```

Purpose:

- expose per-account classic balances (native XLM + trustlines) for the account
  detail page without re-derivation on each request
- answer per-asset holder queries via `idx_abc_asset`

Design notes:

- `asset_type` is the XDR `AssetType` `SMALLINT` enum (`0=native`,
  `1=credit_alphanum4`, `2=credit_alphanum12`) per
  [ADR 0031](../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md); the
  parser never persists `pool_share (3)` here because pool-share trustlines are
  redirected to `lp_positions` at staging time
- native rows leave `asset_code` / `issuer_id` NULL; `ck_abc_native` closes the
  NULL-in-UNIQUE loophole and the pair of partial unique indexes ensures exactly one
  row per logical asset per account

### 4.18 ~~Account Balance History~~ (dropped)

Per [ADR 0035](../../../lore/2-adrs/0035_drop-account-balance-history.md)
(accepted) / task 0159 (completed), the `account_balance_history` table has
been dropped: its only intended consumer was a "balance over time" chart
endpoint that is deferred indefinitely, and the DB was carrying ~90 GB of
unread partitioned data at 11 M-ledger projection.

Current balance state lives entirely in `account_balances_current` (§4.17).
If the chart feature is re-scoped in the future, a dedicated ADR will define
a new materialisation shape (append-only vs daily rollup, retention window,
etc.); the old table shape is not the assumed starting point.

Migration `0007_account_balances.sql` no longer creates the table; the
indexer write path and domain types were trimmed in the 0159 PR.

## 5. Relationships and Data Flow

### 5.1 Ingestion Flow into the Schema

The schema is populated by the Galexie-based ingestion pipeline described in the main
technical design.

At a high level:

- one ledger close produces one ledger record
- each ledger produces many transaction records
- each transaction may produce operations, contract invocations, and events
- derived explorer entities such as assets, accounts, NFTs, and liquidity pools are updated
  from extracted state and known event patterns
- liquidity pool snapshots are appended as time-series records for chart-oriented reads

### 5.2 Child-Entity Lifecycle

The schema models a parent-child structure where appropriate:

- deleting a transaction cascades through `operations_appearances`, `transaction_participants`,
  `soroban_events_appearances`, `soroban_invocations_appearances`, and `nft_ownership`
  via the composite `(transaction_id, created_at)` FK
- contract-linked entities remain queryable through `soroban_contracts.id` BIGINT FK
  relationships; joining back to the natural StrKey uses the `contract_id` UNIQUE column

### 5.3 Public Lookup Keys vs Internal Keys

The model combines public identifiers with internal surrogate keys under the two
surrogate-key ADRs ([0026](../../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md)
for accounts, [0030](../../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md)
for Soroban contracts):

- **Public lookup keys**: `ledgers.sequence`, `transactions.hash`,
  `soroban_contracts.contract_id`, `accounts.account_id`, `liquidity_pools.pool_id`.
  These are what API routes, URLs, and responses carry.
- **Internal join keys**: `BIGSERIAL id` surrogates on `accounts`, `soroban_contracts`,
  `transactions`, and every partitioned child's `(id, created_at)` composite PK. Every
  FK column references a surrogate `id`, never the StrKey.

Pattern A — request boundary resolution: API routes that take a StrKey parameter
resolve it to the surrogate via the unique index on the natural key before running
the main query (`SELECT id FROM accounts WHERE account_id = $1`).

Pattern B — response boundary join: responses that display a StrKey join back to
`accounts` / `soroban_contracts` once at the end.

The public API surface is unchanged by this rewrite. Hex hashes (ADR 0024) and enum
strings (ADR 0031) are also rendered at the serialization layer, not at the DB.

## 6. Indexing, Partitioning, and Retention

### 6.1 Indexing Strategy

The current schema uses indexes for four main reasons:

- fast public lookup by canonical identifier
- efficient recent-history access by time or ledger order
- selective GIN / trigram access for variable-shaped or free-text fields
- partial uniqueness for identity constraints that depend on a row's type

Notable patterns in the current design:

- **Identity indexes**: `ledgers.hash` (unique), `transaction_hash_index.hash`
  (uniqueness for partitioned `transactions` via the proxy table)
- **Time-oriented indexes**: `idx_ledgers_closed_at`, `idx_accounts_last_seen`,
  `idx_tx_source_created`, `idx_lps_pool`, etc. — descending on the time column
  for recent-first browsing
- **GIN / trigram**: `idx_contracts_search` (full-text on `soroban_contracts.search_vector`),
  `idx_assets_code_trgm` (trigram on `assets.asset_code`),
  `idx_nfts_name_trgm` (trigram on `nfts.name`)
- **Partial uniqueness**: `uidx_assets_native` / `uidx_assets_classic_asset` /
  `uidx_assets_soroban` (one row per logical asset depending on `asset_type`),
  `uidx_abc_native` / `uidx_abc_credit` on `account_balances_current`
- **Prefix-search btree**: `idx_accounts_prefix` / `idx_contracts_prefix` using
  `text_pattern_ops` so that `LIKE 'G...%'` queries on the StrKey are index-driven
- **Filtered partial indexes** for rarely-NULL columns: `idx_lpp_shares`,
  `idx_contracts_wasm`. (Former `idx_ops_contract` / `idx_ops_pool` /
  `idx_ops_destination` dropped in task 0163 — the wide `uq_ops_app_identity`
  UNIQUE on `operations_appearances` serves their leftmost-prefix lookups;
  reversible if telemetry demands it.)

Column-type choices also affect indexing economics: `BYTEA(32)` hashes
([ADR 0024](../../../lore/2-adrs/0024_hashes-bytea-binary-storage.md)) and `SMALLINT`
enum columns ([ADR 0031](../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md))
each cut index size compared to the VARCHAR originals, which is material at mainnet-year
volumes.

### 6.2 Partitioning Strategy

Per [ADR 0027](../../../lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md),
all high-volume child tables are partitioned by month on `created_at`; lightweight
anchor and registry tables stay unpartitioned:

- **Partitioned (`RANGE (created_at)` monthly):** `transactions`, `operations_appearances`,
  `transaction_participants`, `soroban_events_appearances`,
  `soroban_invocations_appearances`, `liquidity_pool_snapshots`,
  `nft_ownership`
- **Unpartitioned:** `ledgers`, `transaction_hash_index`, `accounts`,
  `soroban_contracts`, `wasm_interface_metadata`, `assets`, `nfts`,
  `liquidity_pools`, `lp_positions`, `account_balances_current`

Partition creation is handled by a dedicated partition-management Lambda
(`crates/db-partition-mgmt`, see task 0139); partitions follow the
`<table>_y{YYYY}m{MM}` naming convention (e.g. `operations_y2026m04`) and are
provisioned ahead of the leading edge. Partitioning keeps retention, maintenance,
and time-sliced reads practical on the high-write tables.

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

Per [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)
the schema **does not** persist raw XDR payloads. Heavy read fields (full envelope,
result, result-meta, parsed contract events, full invocation tree) are fetched at
request time from the public Stellar ledger archive and parsed on demand — primarily
for `/transactions/:hash` (E3) and `/contracts/:id/events` (E14).

The DB therefore holds only:

- **Typed summary columns** needed by list endpoints and partition-pruned reads
  (e.g. `operations_appearances.type`, `operations_appearances.amount`, `operations_appearances.asset_code`,
  `transactions.successful`, `transactions.has_soroban`)
- **Appearance indexes** that point to `(transaction, ledger)` tuples for
  contract-centric reads (`soroban_events_appearances`, `soroban_invocations_appearances`)
- **Derived time-series** that answer chart endpoints without re-derivation
  (`liquidity_pool_snapshots`). The parallel `account_balance_history` table
  was dropped per [ADR 0035](../../../lore/2-adrs/0035_drop-account-balance-history.md)
  because its only consumer (a balance-over-time chart endpoint) is deferred;
  it will be re-introduced under a fresh ADR if the feature is scheduled
- **Current-state registries** populated by the ingest pipeline + async enrichment
  workers (`assets`, `nfts`, `soroban_contracts`, `wasm_interface_metadata`,
  `account_balances_current`, `lp_positions`)

This split — typed summaries in the DB, heavy payloads fetched on-demand from the
public archive — is the core architectural choice, not accidental duplication.

## 8. Evolution Rules and Delivery Notes

### 8.1 Schema Evolution Rules

Any future schema change should preserve the same general discipline:

- add new tables or columns only when tied to a documented explorer or ingestion need
- avoid replacing explicit relational structure with oversized generic JSON blobs
- keep public lookup keys stable where routes or API contracts depend on them
- update the general overview first if the conceptual schema changes materially

### 8.2 Current Workspace State

The repository now provides concrete DDL for every table in §4 under
`crates/db/migrations/0001_*` through `0007_*` plus subsequent dated migrations for
replay-safe uniqueness (`20260421*`), enum label helpers (`20260422000000_*`), and
in-place enum-variant additions (`20260422000100_*`). Runtime persistence lives in
the indexer (`crates/indexer/src/handler/persist/`) and follows the 14-step
`persist_ledger` pipeline per
[ADR 0027](../../../lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md).

This document is the detailed schema reference; the narrative
[`technical-design-general-overview.md`](../technical-design-general-overview.md)
remains the primary source of truth for cross-component behavior, and the
`lore/2-adrs/**` trail is authoritative for the "why" behind any individual schema
decision. Per
[ADR 0032](../../../lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md),
any future schema change must also update this file in the same PR.
