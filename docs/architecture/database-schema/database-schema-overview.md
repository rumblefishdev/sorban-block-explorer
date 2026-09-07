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

This document covers the **logical** design of the schema — entities, keys,
relationships, field allocation, and partitioning strategy — which is store-agnostic.
It does not redefine frontend behavior, backend transport concerns, or infrastructure
provisioning except where those influence schema decisions.

**Store: ClickHouse.** Postgres was retired (task 0244); ClickHouse is the sole
production store. The authoritative **physical** schema (engines, column types,
partition keys) is `crates/db-clickhouse/schema/init.sql`, documented in
[`clickhouse-pilot.md`](./clickhouse-pilot.md) with the full PG→CH type-translation
table and divergence rationale. The `CREATE TABLE` blocks in §4 below are retained in
their historical PostgreSQL notation to describe table **shape** readably — for the
live ClickHouse form (`ReplacingMergeTree`, `Int64` surrogates, `Decimal128(7)`,
`intDiv(ledger_sequence, 500000)` partitioning) defer to the authority. The narrative
[`technical-design-general-overview.md`](../technical-design-general-overview.md)
takes precedence for cross-component behavior; per
[ADR 0032](../../../lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md) this
file and `init.sql` are updated together on any schema change.

## 2. Ownership and Design Goals

The block explorer owns its full schema. All chain data is stored in ClickHouse; there is
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
  metadata shapes (`soroban_contracts.metadata`, `wasm_interface_metadata.metadata`).
  Detail-only NFT attributes (formerly `nfts.metadata` JSONB) are NOT persisted —
  per ADR 0043 they are fetched at request time on `GET /v1/nfts/:id` via
  `runtime_enrichment::nft_token_uri` (Soroban RPC `token_uri()` + IPFS gateway,
  LRU 24h, fail-soft). The column was dropped in migration
  `20260507120000_drop_nfts_metadata.up.sql` (task 0195 §2d).
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
by the ClickHouse schema (`crates/db-clickhouse/schema/init.sql`).

Backbone timeline:

- `ledgers` — ledger-close timeline (anchor)
- `transactions` — primary explorer activity entity (partitioned by `created_at`)
- `transaction_hash_index` — unpartitioned hash-to-ledger lookup for direct detail routes
- `operations_appearances` — transaction-scoped appearance index for classic and
  mixed transaction inspection (partitioned; per-op detail recovered from XDR on
  demand per task 0163)
- `transaction_participants` — derived participant links for account-history reads (partitioned)
- `operation_asset_appearances` — per-(asset, transaction) presence index powering
  `/assets/:id/transactions` (task 0359; the asset-dimension twin of
  `transaction_participants`, keyed asset-first; native XLM is a first-class
  surrogate, not absence)
- `operation_pools` — per-(pool, transaction) presence index powering
  `/liquidity-pools/:id/transactions` (task 0365; the pool-dimension twin of
  `transaction_participants`, keyed pool-first; `pool_id` is the raw 32-byte pool
  hash — the same value `operations_appearances.pool_ids` stores per crossing)
- `lp_operation_amounts` — per-(operation, pool, asset) amounts behind that
  endpoint's "Amount" column (task 0279 / issue #371; the value twin of
  `operation_pools`, same pool-leading prefix). `amount` is raw stroops in a
  signed `Int64`, positive when the asset entered the pool — so the sign pattern
  names the event (trade `+/-`, deposit `+/+`, withdrawal `-/-`) without a type
  column. Rows are per-op sums of the op's claim atoms, never per atom

Soroban activity model (per ADRs 0033/0034 these are pure appearance indexes — parsed
contract-event and invocation-tree payloads are fetched at read time from the public
Stellar archive, not stored in the DB):

- `soroban_contracts` — deployed contracts (`BIGSERIAL id` + `VARCHAR(56)` natural `contract_id`)
- `wasm_interface_metadata` — WASM ABI keyed by `wasm_hash`
- `soroban_events_appearances` — contract-event appearance index (partitioned)
- `soroban_invocations_appearances` — contract-invocation appearance index (partitioned)

Derived explorer entities:

- `assets` — unified asset registry (native, classic_credit, Soroban SEP-41);
  a SAC is a facet of its classic_credit / native row, not a separate type
  (ADR 0051 / task 0339). Renamed from `tokens` in ADR 0036 / task 0154
- `accounts` — account identity hub (`BIGSERIAL id` surrogate + `VARCHAR(56)` natural `account_id`)
- `account_balances_current` — classic trustline current balances (history table dropped per ADR 0035)
- `balances` — unified per-holder balances for ALL asset types (task 0331 Option C, CH-only); raw
  `Int128` `amount` scaled by `decimals` at read; classic dual-written from
  `account_balances_current`, type-3 from `ContractData Balance(Address)` ledger state.
  `holder_id = cityhash64(holder StrKey)` (G-account or C-contract) in the one surrogate space
  shared with `accounts.id` / `soroban_contracts.id`; resolve back to a StrKey via `accounts` (G) /
  `soroban_contracts` (C) — there is no dedicated address dimension.
  **Lifecycle** (ADR 0055 / task 0463): `closed_at_ledger Int64 DEFAULT 0` — `0` while the holding
  relationship is live, otherwise the ledger the entry disappeared in; rows are never deleted. A
  live-but-empty holding and a removed one both write `amount = 0`, so the amount alone CANNOT
  carry liveness — the read path filters `closed_at_ledger = 0`, never `amount != 0` (shipped; a
  guard test pins the predicate so it cannot regress). The same column answers whether an ACCOUNT
  still exists: native XLM lives on the `AccountEntry`, so the native row's `closed_at_ledger`
  is the account's own lifecycle, and `deleted` is read from it rather than derived from
  operation history (task 0500). The writer
  stamps closures exactly; rows seeded from the checkpoint snapshot carry the run's checkpoint
  ledger, meaning "closed at or before"
- `account_entry_state` — signers + thresholds per account (task 0463, issue #377): one row per
  account, the FULL signer set as parallel arrays (`signer_keys/weights/types`), plus
  `master_weight` + `threshold_low/med/high` + account `flags`; RMT(`last_updated_ledger`) keyed on
  `account_id` — whole-set replacement so removed signers cannot ghost. Raw-XDR truth: the master
  key is NOT in the signer list (its weight is thresholds byte 0); Horizon synthesizes a master
  entry, we must not. A row-less account does NOT mean thin coverage: the checkpoint seed wrote
  state for every live account, so a missing row means the account has no ledger entry (probed:
  450 of 450 such accounts ABSENT via `getLedgerEntries`). The one shape that WOULD be a gap —
  a live CLASSIC holding shown with no signing configuration — measures zero and is what the
  page warns on. A Soroban holding is not that shape: it outlives `account_merge`, so it says
  nothing about whether an account exists.
  **Semantics: LAST KNOWN configuration, not liveness.** The writer emits nothing on
  `account_merge` (a merge cannot change signers), and RMT cannot delete, so a merged account
  keeps its final signer set — deliberately, the same way `accounts` keeps merged accounts.
  11,639 rows are in that state today and the share grows with pubnet merge churn. Liveness
  comes from the native holding's `closed_at_ledger` (ADR 0055); an aggregate over this table
  ALONE ("how many accounts are multisig?") silently counts dead accounts
- `balance_aggregates` (+ refreshable MV) — pre-computed per-`asset_id` `total_supply` (`sum`) /
  `holder_count` (`countIf(amount > 0)`) over `balances`
- `asset_aggregates` / `soroban_token_supply` — **DROPPED (task 0331)**. Classic supply/holders now
  flow through `balance_aggregates` over the unified `balances`; `total_supply = sum(amount)` is the
  SOLE supply source for ALL asset types (Option A — no per-token `TotalSupply` key read). The
  `balances` family is ClickHouse-only (see `clickhouse-pilot.md §4f`); there is no
  `soroban_token_balances` / `soroban_asset_aggregates` (superseded by the unified model on the pivot)
- `nfts`, `nft_ownership` — NFT registry plus partitioned ownership history
- `liquidity_pools`, `liquidity_pool_snapshots`, `lp_positions` — classic LP state +
  time-series snapshots + per-account share positions
- `liquidity_pools` is also the dimension for **Soroban AMM pools** (ADR 0058,
  task 0374): `pool_kind = 1` rows discovered from router `add_pool` events,
  carrying `legs Array(Int64)` (token-contract surrogates, 2–4 legs),
  `deployment_id` (registering router), `pool_type_raw` and `fee_bps`.
  Registration provenance (the subpool salt, raw `init_args` beyond the fee)
  is deliberately NOT materialised — the `add_pool` event sits complete in
  `soroban_events`; extract on demand, never copy. Their id bytes are a
  CONTRACT address payload (`C...` on the wire, never `L...`)
- Soroswap pairs (task 0518) are `pool_kind = 1` rows in the same registry —
  `deployment_id` = the factory, `fee_bps` = the vendor's compiled-in 30,
  `pool_type_raw` empty (the vendor emits no type). The pair is its own LP
  token: in `pool_instance_state` its `plane_id` and `share_token_id` are
  the pair itself, `total_shares` from the SEP-41 `TotalSupply`.
- Config-factory (Phoenix-family) pools (task 0518, third adapter) are
  `pool_kind = 1` rows too — `deployment_id` = the factory,
  `fee_bps` = the pool's own per-pool `total_fee_bps` (creation-time
  snapshot; mutable on chain), `pool_type_raw` = the vendor `PairType`
  discriminant verbatim ("0" = XYK). The share token is a SEPARATE
  contract (like Aquarius, unlike Soroswap): `pool_instance_state` rows
  stage only on CONFIG writes, `share_token_id` from CONFIG, and
  `total_shares` there is STRUCTURALLY 0 forever for this family (the
  supply key never co-occurs with a post-creation CONFIG write) — the live
  supply is the share token's own tracked supply on the generic token
  pipeline (a per-op row would clobber the RMT whole-row key). A reader
  must never render that 0 as a measured value.
- `pool_state_changes` — Soroban pool reserves, ONE row per
  `(pool, plane, ledger)` (the plane is in the key so a forged plane write
  cannot evict a genuine row — see the DDL comment),
  `reserves Array(Int128)` verbatim. Same grain and
  mechanism as the classic snapshots: a version-less `ReplacingMergeTree`
  whose determinism comes from folding at stage time, never from a version
  column. Fungible pools write plane `PoolData`; concentrated pools write
  `Reserve0/1` on their own instance, so both writers feed this table and the
  fold spans them. `plane_id` records WHICH plane wrote the row — reads must
  filter on it (below). The target state-fact shape — classic snapshot
  history joins INTO it if the snapshot models unify (ADR 0058 §3)
- `pool_instance_state` — what a pool declares ABOUT ITSELF, read from its own
  instance storage (side table, `asset_sac` pattern; ADR 0058 §4): `plane_id`
  and `share_token_id`, versioned by sighting ledger. The pool contract is the
  ledger-authenticated owner of that entry, which is what makes it usable as
  an authority. A Soroban token IS an LP share exactly when it appears here
  (`share_token_id = 0` is structural for concentrated pools, which never mint
  one)

**Reserve provenance is a READ-TIME predicate, not just a stored column.** A
plane entry names its pool in a key payload the writing contract chooses
freely, so any contract can publish reserves under another pool's id. Reads of
`pool_state_changes` therefore keep only rows whose `plane_id` matches the
plane the pool itself declares in `pool_instance_state`; the same rule gates
the pool list's activity ordering. Symmetrically, a registration is only
written when the named pool's instance declares the emitter as its `Router`
(see the indexing-pipeline overview).

High-level relationship sketch:

```text
ledgers
  └─ transactions (partitioned)
       ├─ operations_appearances (partitioned)
       ├─ transaction_participants (partitioned)
       ├─ operation_asset_appearances (partitioned)
       ├─ operation_pools (partitioned)
       ├─ lp_operation_amounts (partitioned)
       ├─ soroban_events_appearances (partitioned)
       └─ soroban_invocations_appearances (partitioned)

soroban_contracts
  ├─ wasm_interface_metadata
  ├─ soroban_events_appearances
  ├─ soroban_invocations_appearances
  ├─ assets
  └─ nfts ─ nft_ownership (partitioned)

liquidity_pools                       # classic (pool_kind=0) + soroban AMM (pool_kind=1)
  ├─ liquidity_pool_snapshots (partitioned)   # classic only
  ├─ lp_positions                             # classic only
  ├─ pool_state_changes (partitioned)         # soroban reserves, one row per (pool, ledger)
  └─ pool_instance_state                      # soroban pool's own declaration: plane + share token

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
    source_id         BIGINT               REFERENCES accounts(id),  -- ADR 0026 surrogate; NULLable for Variant A parse_error tx (lore-0209)
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
  displayed `G...` StrKey is obtained via JOIN back to `accounts.account_id`.
  The column is `NULL`able to accommodate Variant A `parse_error` transactions
  whose envelope was not decodable and therefore carry no known source
  (lore-0209). Query paths that surface such rows (transaction list/detail,
  ledger-scoped transaction listing) use `LEFT JOIN accounts`; paths that
  drive through operations / events / liquidity pools / assets never match
  parse_error tx (no rows in those tables) and keep plain `JOIN`.
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
    application_order SMALLINT,                                            -- task 0192: 1-based MIN apply pos across folded ops
    ledger_sequence   BIGINT       NOT NULL,
    created_at        TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE,
    CONSTRAINT ck_ops_app_pool_id_len CHECK (pool_id IS NULL OR octet_length(pool_id) = 32),
    CONSTRAINT ck_ops_app_type_range  CHECK (type BETWEEN 0 AND 127),      -- ADR 0031 range
    CONSTRAINT ck_ops_app_amount_pos  CHECK (amount > 0),
    CONSTRAINT ck_ops_app_application_order_range
        CHECK (application_order IS NULL OR (application_order BETWEEN 1 AND 32767)),
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
  deferred FK attached once `liquidity_pools` exists in migration 0006.
  **CH divergence (task 0261/0268):** the ClickHouse parallel store replaces
  the scalar with `pool_ids Array(FixedString(32))` — path payments record
  every pool crossed by their result claim atoms (multi-hop lossless), LP
  deposit/withdraw a single element, `[]` = no pool. PG keeps the legacy
  scalar (path payments stay NULL) pending its retirement
- composite `(id, created_at)` PK is required because the partition key must be in
  every unique index on a partitioned table; `created_at` is inherited verbatim from
  the parent transaction so per-partition cascade is well-defined
- `uq_ops_app_identity` uses PG 15+ `NULLS NOT DISTINCT` so NULL-heavy shapes
  (e.g. type-14 `CREATE_CLAIMABLE_BALANCE` with source inherited from tx)
  collapse correctly. Observed compression: 28% overall on backfill sample,
  type-14 collapses from 12 709 operations to 179 rows
- `transfer_amount NUMERIC(28,7)` was dropped — no API endpoint reads it, and
  per-op detail is already re-materialised from XDR by
  `runtime_enrichment::stellar_archive` extractors per
  [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)
- `application_order SMALLINT` was dropped together with `transfer_amount` in
  task 0163 on the premise "no API endpoint reads it", and re-introduced by
  [task 0192](../../../lore/1-tasks/active/0192_BUG_operations-appearances-ordering-not-apply-order.md)
  after empirical evidence showed endpoint 03 Statement C had implicitly
  re-introduced an ordering dependency through `ORDER BY oa.id`. The column
  carries the 1-based on-chain apply position; for folded rows (multiple
  identical-identity envelope ops collapsed into one row) it stores the
  MIN of the folded ops' indices — the position of the row's first
  occurrence in `tx.operations[]`. NULLABLE for backward compatibility
  with pre-task-0192 historical rows
- ingest staging aggregates operations at the
  `HashMap<OpIdentity, (count, min_apply_order)>` level before the bulk
  INSERT, with `min_apply_order` tracked via explicit `min()` reduction so
  the value is independent of HashMap iteration order. The pre-task-0192
  alphabetic-identity sort that produced the ordering bug
  (`oa.id` BIGSERIAL alphabetic-by-asset_code on multi-asset bulk txs)
  has been replaced with `sort_by_key((tx_hash_hex, application_order))`.
  Write layer uses `ON CONFLICT ON CONSTRAINT uq_ops_app_identity DO NOTHING`
  for replay idempotency

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

### 4.5.1 Operation Asset Appearances (task 0359)

ClickHouse-only (like the `balances` family; see `clickhouse-pilot.md`). The
**asset-dimension twin of `transaction_participants`** — a per-(asset, transaction)
presence index so a per-asset activity page is a PK-prefix seek.

```sql
CREATE TABLE operation_asset_appearances (
    asset_id        Int64,   -- ids::asset_id surrogate; native = ids::asset_id(0,'',0,0)
    ledger_sequence Int64,
    transaction_id  Int64,
    INDEX idx_oaa_transaction_id transaction_id TYPE bloom_filter(0.001) GRANULARITY 1
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (asset_id, ledger_sequence, transaction_id);
```

> **Read-path.** The tx-list value read filters `(ledger, tx)` on this
> `asset_id`-leading table — not a prefix seek, so unaided it is a partition scan
> (~26 M rows/page on a full partition). Mitigated by the `idx_oaa_transaction_id` > **bloom skip index** (task 0393), which prunes granules holding none of a page's
> tx_ids (~10×); same pattern as `idx_oa_contract_id`. A projection is not a
> candidate (CH 26.3 refuses projections on a ReplacingMergeTree, and a `(ledger,
tx)`-ordered companion would re-store the incompressible `transaction_id` ~85 GiB)
> **`net_settled` was REMOVED (2026-09-04).** The per-(transaction, asset) aggregate
> `max(Σ+, Σ−)` carried no direction and no account, so on an account page an
> inbound and an outbound transfer rendered identically. It is replaced by a
> lossless per-transfer design; the reducer that produced it
> (`persist::stage::ledger_deltas_net_settled` over `xdr_parser::ledger_balance_deltas`)
> is kept, because the authoritative LEDGER balance deltas it reads are the input
> the replacement needs.

Purpose / design notes:

- Fixes the single-asset-slot loss on `operations_appearances`: offers stored ZERO
  assets, path payments kept only `destAsset`, and native XLM was an empty-string
  sentinel. Here **every asset an op touches** is one row, keyed **asset-first** so
  `/assets/:id/transactions` is a bounded seek (not a non-leading density-scan).
- **Pure presence** — no `role` / `application_order` / `amount` / `pool_id`.
  Duplicate (asset, tx) rows within a tx are deduped at write (per-tx set) and
  collapse in the RMT; the read also applies `LIMIT 1 BY (ledger, tx)`.
- **Native XLM is first-class**: `ids::asset_id(0,'',0,0)` (a stable non-zero
  surrogate), never absence — so native has a real per-asset page.
- Populated by the shared parse path (live ingest + the archive backfill run the
  same `emit_asset_appearances`), from two grains: **body** (asset fields on the op
  struct) and **meta** (claimable-balance / LP assets recovered from the same-op
  `LedgerEntryChanges`). Failed txs keep their body-declared assets (parity with
  `operations_appearances`); meta assets are naturally absent for a failed op.
- **Backfill dependency**: needs the Soroban-era XDR re-parse to populate history;
  run it in the SAME rollout as the read swap or the endpoint shows only
  post-deploy classic activity.

### 4.5.2 Operation Pools (task 0365)

ClickHouse-only. The **pool-dimension twin of `transaction_participants`** — a
per-(pool, transaction) presence index so `/liquidity-pools/:id/transactions` is a
PK-prefix seek instead of the density-dependent `has(pool_ids, X)` scan over
`operations_appearances` (the 0281-C read-in-order driver, superseded).

```sql
CREATE TABLE operation_pools (
    pool_id         FixedString(32),  -- raw 32-byte pool hash (no surrogate)
    ledger_sequence Int64,
    transaction_id  Int64
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (pool_id, ledger_sequence, transaction_id);
```

Purpose / design notes:

- `pool_id` is the raw 32-byte hash — the exact value `operations_appearances.pool_ids`
  stores per crossing — so no surrogate resolution is needed (unlike the asset twin).
- **Pure presence** — no `role` / `application_order` / `amount`. Duplicate (pool, tx)
  rows within a tx are deduped at write (per-tx set) and collapse in the RMT; the read
  also applies `LIMIT 1 BY (ledger, tx)`.
- Populated by the indexer as a per-op Rust fan-out over each op's `pool_ids`,
  written beside `transaction_participants` / `operation_asset_appearances`. (The
  Path B backfill below re-keys history via `arrayJoin(pool_ids)`.)
- **Backfill (task 0365 Path B)**: unlike the asset twin, the source `pool_ids` is
  already in ClickHouse, so history is backfilled by a plain CH re-key
  (`INSERT … SELECT arrayJoin(pool_ids), ledger_sequence, transaction_id
FROM operations_appearances`) — no XDR re-parse.

### 4.5.3 LP Operation Amounts (task 0279)

ClickHouse-only. The **value twin of `operation_pools`** — what each operation
actually moved through a pool, so `/liquidity-pools/:id/transactions` can render
an "Amount" column (`12,059.29 XLM → 38.5M KALE`) instead of a bare event chip
(issue #371). `operation_pools` remains the paging driver; this table is the
value lookup for the page's `(ledger, tx)` set.

```sql
CREATE TABLE lp_operation_amounts (
    pool_id           FixedString(32),  -- raw 32-byte pool hash (no surrogate)
    ledger_sequence   Int64,
    transaction_id    Int64,
    application_order Int16,            -- op position within the tx
    asset_id          Int64,            -- ids::asset_id surrogate (native first-class)
    amount            Int64             -- raw stroops, SIGNED from the pool's side
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (pool_id, ledger_sequence, transaction_id, application_order, asset_id);
```

Purpose / design notes:

- **Row grain is (operation, pool, asset)** — the op's claim atoms are summed in
  Rust before the insert, NOT written per atom. One op can take the same pool
  several times (CAP-38 interleaved order-book/AMM matching) and every such atom
  carries the identical ORDER BY tuple, so per-atom rows would have the RMT keep
  one and silently drop the rest of the fill. A per-op sum is deterministic on
  replay, so live ingest and the historical re-parse emit identical rows.
- **Sign carries the semantics**: positive = the asset entered the pool, negative
  = it left. Trade `+/-`, deposit `+/+`, withdrawal `-/-`; the two rows of one
  (op, pool) are its two legs. No event-type column, no unsigned + direction pair.
- **`Int64` raw stroops**, scaled by 7 at read like `balances`:
  classic AMM pools are 7-decimal by definition, the XDR sources are `int64`, and
  a per-op sum is bounded by the pool's own `int64` reserve. Not `Int128` (that
  width serves Soroban i128 token amounts, unreachable for a classic pool) and
  not `Decimal128(7)` (the read-model choice in `liquidity_pool_snapshots`).
- **Two producers, one shape**: trades from the claim atoms
  `persist::stage::gross_volume_a_by_pool` already walks (it sums `amountA` away —
  this table keeps that value), deposits/withdrawals from the op's own
  `LedgerEntryChanges` (they carry no claim atoms; the op body holds the caller's
  max/min bounds, not what actually moved).
- **Backfill**: a targeted XDR re-parse — the per-pool amounts were never
  persisted, so unlike `operation_pools` there is no CH-side re-key. Scope is the
  ~13.15M ledgers with pool activity (`SELECT DISTINCT ledger_sequence FROM
operation_pools`), ~20.6% of history. Additive: no existing table is touched, the
  indexer keeps running, rollback is `DROP TABLE`. Validation gate:
  `sum(abs(amount))` over the asset-A legs per (pool, ledger) must equal
  `liquidity_pool_snapshots.gross_volume_a` — both derive from the same atoms.
  ABS because `gross_volume_a` is gross: it counts each atom's A-side amount
  whichever way the swap went, so a ledger where the pool only sold A has every
  A leg negative here. An op crossing one pool in both directions nets out at
  this table's per-op grain and is a known, legitimate mismatch.
- No skip index: every read is a `pool_id` PK-prefix seek.

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
    name                    VARCHAR(256),                                   -- ADR 0042; legacy/empirically empty — on-chain token name lives in instance-storage METADATA, see task 0297
    search_vector           TSVECTOR GENERATED ALWAYS AS (
                                to_tsvector('simple', COALESCE(name, '') || ' ' || contract_id)
                            ) STORED,
    CONSTRAINT ck_sc_wasm_hash_len       CHECK (wasm_hash IS NULL OR octet_length(wasm_hash) = 32),
    CONSTRAINT ck_sc_contract_type_range CHECK (contract_type IS NULL OR contract_type BETWEEN 0 AND 15)
);
CREATE INDEX idx_contracts_type   ON soroban_contracts (contract_type);
CREATE INDEX idx_contracts_wasm   ON soroban_contracts (wasm_hash) WHERE wasm_hash IS NOT NULL;
CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
CREATE INDEX idx_contracts_prefix ON soroban_contracts (contract_id text_pattern_ops);
```

> **On-chain token metadata (task 0297, ClickHouse).** `name` / `symbol` /
> `decimals` for Soroban tokens are on-ledger in the contract's instance storage
> under `Symbol("METADATA")` (a `{decimal, name, symbol}` struct — NOT a
> standalone `Symbol("name")` entry; the `name` column above is empirically
> empty). On the CH datastore the parser recovers them into a dedicated side
> table `soroban_contract_metadata(contract_id, name, symbol, decimals, version)`
> — `ReplacingMergeTree(version)`, key `contract_id` — written by the indexer
> (`created` + `updated`, SACs skipped) and composed at read (`LEFT JOIN`;
> `decimals` defaults to 7 for classic/SAC). It is a separate table, not columns
> on `soroban_contracts`: RMT whole-row replace + that table's multiple writers
> would clobber in-row metadata, and identity vs metadata update on different
> clocks. The API exposes `name`/`symbol`/`decimals` on the contract-detail and
> asset detail/list responses.

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
- `deployer_id` is an `accounts.id` surrogate FK (ADR 0026). The attributed
  account is the **operation-level effective source** of the
  `CreateContract*` host function: `op.source_account` when the op
  carries an explicit per-op source override, otherwise the inner
  `tx.source_account` (fee-bump `feeSource` is never used). For
  factory-pattern deploys where the `CreateContract*` lives inside an
  `InvokeContract` auth tree, the deployer is the signer of the
  enclosing `SorobanAuthorizationEntry` —
  `SourceAccount` credentials inherit the effective op source; explicit
  `Address(Account)` credentials carry the signer directly;
  contract-signed credentials yield no human deployer (the call site
  falls back to tx source). Built by
  `xdr_parser::extract_op_source_per_contract`, threaded into
  `extract_contract_deployments` via the `deployer_by_contract` override
  map. Task 0255 Phase 1; prior parser revisions stored the inner-tx
  source unconditionally and misattributed the ~12 % of mainnet deploys
  with a per-op override
- `contract_type` is `SMALLINT` backed by the Rust `ContractType` enum (ADR 0031);
  nullable because the two-pass upsert in `persist/write.rs` registers bare StrKey
  references before deployment meta is observed — those rows start NULL and get
  filled when the deploy meta lands. The `contract_type_name(ty)` SQL helper renders
  the canonical string
- `name` is `VARCHAR(256)` ([ADR 0042](../../../lore/2-adrs/0042_soroban-contracts-typed-name-column.md))
  populated by the indexer from the standard `Symbol("name")` ContractData
  persistent storage entry — written at deploy time when the storage init
  is in the same ledger (constructor pattern), or backfilled by
  `apply_contract_name_writes` on the next ledger that emits the storage
  Created/Updated event (deploy-then-init pattern). NULL for contracts
  that never store a `Symbol("name")` value (generic dApps, libraries,
  routers). Replaces the previous `metadata JSONB` blob, which carried a
  single closed-shape field and so failed the typed-columns-vs-JSONB test
  established in ADR 0023 and codified in ADR 0037
- `search_vector` (PG-era generated TSVECTOR) was retired with the store —
  it does not exist in the CH table. Contract search matches the typed
  `name` and the StrKey directly (`positionCaseInsensitive` scans in
  `crates/api/src/search/queries.rs`)
- `is_sac` is set to `true` for every SAC contract observed in scope.
  Three classification paths land here:
  1. **In-window SAC deploy** — `extract_contract_deployments` reads
     `LedgerEntryChange` with `executable=stellar_asset` and stages
     `is_sac=true` directly on the contract row.
  2. **Un-deployed SAC → asset facet, not contract (task 0323 → ADR 0051)** — a
     classic asset's deterministic SAC `contract_id` can surface via a CAP-67
     event with no on-chain deploy. `xdr_parser::detect_undeployed_sac_overrides`
     collects these crypto-proven emitters per ledger
     (`sac_override_from_event_topics`, `emitter == derive_sac(asset)`); on
     the CH path each suppresses the Pass-2 FK stub (**no `soroban_contracts`
     row** is written) and folds the SAC handle onto the underlying
     classic_credit / native `assets` row (`sac_contract_id` set, `sac_deployed
= false`), so `soroban_contracts` holds **deployed instances only**. The legacy PG step
     `apply_sac_overrides_for_skeleton_contracts` still UPDATEs `is_sac=TRUE`
     on a matching skeleton row (task 0218, idempotent via `WHERE is_sac=FALSE`)
     — being deprecated with the PG path.
  3. **(Future)** Soroban RPC `getLedgerEntries` fetch on first
     reference for stragglers neither in-window-deployed nor
     forward-derivable — not implemented; mentioned only to mark
     the design space.

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
    id              SERIAL        PRIMARY KEY,
    asset_type      SMALLINT      NOT NULL,   -- TokenAssetType: 0=native, 1=classic_credit, 3=soroban (2=sac RETIRED, ADR 0051)
    asset_code      VARCHAR(12),
    issuer_id       BIGINT        REFERENCES accounts(id),           -- ADR 0026
    contract_id     BIGINT        REFERENCES soroban_contracts(id),  -- ADR 0030; soroban identity only
    -- IDENTITY ONLY. `name` (task 0304), `total_supply` / `holder_count` /
    -- `icon_url` (task 0310) were dropped: this row is re-written whole every
    -- ledger, so any mutable non-key column gets clobbered. Supply/holders are
    -- aggregated from `balances` into `balance_aggregates` (task 0293/0331);
    -- name/icon live in `asset_enrichment` (ADR 0050) coalesced over
    -- `soroban_contract_metadata` (task 0297).
    CONSTRAINT ck_assets_asset_type_range CHECK (asset_type BETWEEN 0 AND 15),
    -- ADR 0051: a SAC is a FACET of its classic_credit / native asset, not a
    -- separate type. `assets` holds only the asset's IDENTITY; the SAC handle
    -- lives in the `asset_sac` side table (below), NOT as columns here — `assets`
    -- is re-written whole every ledger, so a mutable non-key column would be
    -- clobbered by the next re-emit.
    CONSTRAINT ck_assets_identity CHECK (
        (asset_type = 0 AND asset_code IS NULL     AND issuer_id IS NULL     AND contract_id IS NULL)
     OR (asset_type = 1 AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
     OR (asset_type = 3 AND issuer_id IS NULL      AND contract_id IS NOT NULL)
    )
);
-- partial unique indexes enforce one row per logical asset:
CREATE UNIQUE INDEX uidx_assets_native        ON assets ((asset_type)) WHERE asset_type = 0;
CREATE UNIQUE INDEX uidx_assets_classic_asset ON assets (asset_code, issuer_id) WHERE asset_type = 1;
CREATE UNIQUE INDEX uidx_assets_soroban       ON assets (contract_id)           WHERE asset_type = 3;
CREATE INDEX idx_assets_type      ON assets (asset_type);
CREATE INDEX idx_assets_code_trgm ON assets USING GIN (asset_code gin_trgm_ops);

-- SAC facet side table (ADR 0051). One logical row per SAC-having classic_credit
-- / native asset, keyed byte-for-byte like `assets`, joined at read. Written by
-- the INDEXER only on a SAC sighting (deploy → sac_deployed=1, un-deployed
-- override event → 0), NEVER on a plain trustline re-emit, so the per-ledger
-- whole-row `assets` rewrite can't zero it (the clobber that moved total_supply →
-- asset_aggregates and name/icon → asset_enrichment). On ClickHouse it is an
-- AggregatingMergeTree with SimpleAggregateFunction(max) columns: `sac_deployed`
-- is monotonic, so a deploy sighting `max`-beats any later un-deployed override.
-- The SAC's C… StrKey is NOT stored — it re-derives on read from `code:issuer`.
CREATE TABLE asset_sac (
    asset_type      SMALLINT,
    asset_code      VARCHAR(12),
    issuer_id       BIGINT,
    contract_id     BIGINT,      -- 0 for the classic/native carrier
    sac_contract_id BIGINT,      -- surrogate of the SAC's C… StrKey (max-merged)
    sac_deployed    BOOLEAN      -- deployed on-chain? (max-merged → sticky-true)
    -- CH: ENGINE = AggregatingMergeTree
    --     ORDER BY (asset_type, asset_code, issuer_id, contract_id);
    -- No skip-index on `sac_contract_id`: every read aggregates the whole (small,
    -- ~31k-row) table (`GROUP BY key, max(sac_contract_id)`), so a per-column index
    -- prunes nothing; the `/assets/{C…}` deep-link filters that join result.
    -- Readers by direction: asset → SAC on the LP endpoints and /assets;
    -- SAC → asset (reverse, task 0441) on /contracts + /contracts/:id
    -- (`sac_asset` field), one batched whole-table aggregation per page —
    -- accepted at the measured 7.79 MiB; bloom_filter skip index is the
    -- named upgrade past ~5M rows.
);
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
  identity rules in `ck_assets_identity` close the NULL-in-UNIQUE loophole
- **SAC is a facet, not a type** ([ADR 0051](../../../lore/2-adrs/0051_sac-as-facet-of-classic-credit.md),
  task 0339): a classic asset and its Stellar Asset Contract are the **same
  economic asset**, so SAC-ness is carried in the `asset_sac` side table —
  `sac_contract_id` (surrogate of the SAC's `C…` StrKey — the `C…` itself is NOT
  stored; it re-derives on read from `code:issuer`) + `sac_deployed` — keyed on
  the classic_credit / native identity, never a separate `asset_type = 2`.
  Populated for deployed AND un-deployed SACs (an un-deployed SAC still emits
  events that must resolve to the asset). It lives in a side table (not columns
  on `assets`) because `assets` is re-written whole every ledger. This supersedes
  the classic↔SAC split (and its ADR 0038 native-XLM-SAC carve-out)
- native XLM is uniquely identified by `asset_type = 0`; classic credit by
  `(asset_code, issuer_id)`; Soroban-native assets by `contract_id` via
  `uidx_assets_soroban`
- the native XLM singleton (`asset_type = 0`, name `"Stellar Lumen"`, all
  identity columns NULL) is bootstrapped on **two paths** that both rely on
  `uidx_assets_native`'s `WHERE NOT EXISTS` no-op semantics:
  - **Migration `20260428000000_seed_native_asset_singleton`** seeds the row
    on a clean DB (name = `"Stellar Lumen"`).
  - **Parser path** (task 0219) — `xdr_parser::native_asset_singleton()`
    emits one `ExtractedAsset { asset_type: Native, … }` per ledger from
    `crates/indexer/src/handler/process.rs`; the persist step
    `upsert_assets_native` UPSERTs idempotently against `uidx_assets_native`.
    Belt-and-suspenders coverage for environments where the migration
    seed never ran (e.g. mid-stream restart from a manual schema apply).
- producers per `asset_type`:
  - `0 = Native` → migration seed + `native_asset_singleton()` (above).
  - `1 = ClassicCredit` → `xdr_parser::detect_classic_credit_assets(changes)`
    walks every `trustline` `LedgerEntryChange`, extracts
    `(asset.code, asset.issuer)` from `data.asset` for live changes or
    falls back to `key.asset` for removed changes, dedupes within the
    ledger, and emits one row per distinct pair. `pool_share`
    trustlines are intentionally skipped — those are LP positions,
    handled by `extract_lp_positions`. Producer added in task 0219 to
    close Karol's pre-audit Bug #1 (classic credits had no producer;
    the persist branch fired only in tests).
  - **SAC facet** (`asset_sac` side table, ADR 0051) → a SAC deploy
    (`xdr_parser::detect_assets`, `crates/xdr-parser/src/state.rs`) emits a facet
    row with `sac_deployed = 1`; an un-deployed SAC seen via a CAP-67 event
    (`detect_undeployed_sac_overrides`) emits one with `sac_deployed = 0`. Neither
    emits a distinct `asset_type` row; the staging `push_sac` accumulator
    `max`-merges the facet per key (deploy beats override), mirrored cross-ledger
    by the `asset_sac` AggregatingMergeTree.
  - `3 = Soroban` → `xdr_parser::detect_assets` for non-SAC deployments
    whose WASM interface classifies as `Fungible` via
    `xdr_parser::classify_contract_from_wasm_spec`.
- `icon_url` serves the list-page thumbnail (per-row) and lives in the
  `asset_enrichment` side table (ADR 0050 / task 0231), never on the `assets`
  row itself — the `assets.icon_url` column was dropped in task 0310 after
  measuring 0 populated rows in prod. It is populated by the **type-1
  enrichment worker Lambda** (`crates/enrichment-worker`, task 0191): the
  indexer Lambda emits one SQS message per newly inserted asset, the worker
  consumes the queue, fetches the issuer's
  `https://{home_domain}/.well-known/stellar.toml` via the shared
  `enrichment-shared::sep1` fetcher, extracts the matching
  `CURRENCIES[].image`, and writes back. Worker writes are unconditional —
  duplicate or refresh messages overwrite, which keeps the worker stateless.
  Permanent fetch failures (missing `home_domain`, 4xx, malformed TOML, no
  matching `CURRENCIES[]` row, URL exceeding the column length) write an
  empty-string sentinel `''`, which the un-enriched-asset producer query
  treats as "already tried" — those rows are not re-emitted to the enrichment
  queue. Distinct from **type-2 runtime enrichment** in
  `crates/api/src/runtime_enrichment` (task 0188), which fetches per-request
  for `description` / `home_page` and never writes to the DB.
- asset-detail SEP-1 fields (`description`, `home_page`, `conditions`,
  `is_asset_anchored`, `anchor_*`, `redemption_instructions`,
  `display_decimals`, organisation info) are NOT stored on this row at all —
  they are resolved at request time on `GET /v1/assets/{id}` by the
  `runtime_enrichment::sep1` fetcher (task 0188), which reads
  `accounts.home_domain` for the issuer and pulls `https://{home_domain}/.well-known/stellar.toml`.
  This narrows the original typed-columns plan from
  [ADR 0023](../../../lore/2-adrs/0023_tokens-typed-metadata-columns.md) Part 3
  and supersedes the per-entity S3 hydration sketched under task 0164;
  details-only fields are not persisted at all
- `total_supply` and `holder_count` are stock fields populated by the **indexer per ledger**, not by enrichment Lambda 2 — both are on-chain-derivable from `account_balances_current` (per [ADR 0043](../../../lore/2-adrs/0043_field-allocation-rule.md), list-endpoint + on-chain → indexer). After the credit-balance upsert pass, `recompute_asset_aggregates` (`crates/indexer/src/handler/persist/write.rs`) collects every `(asset_code, issuer_id)` pair touched by this ledger and runs a single UPDATE that rewrites `holder_count = COUNT(*) FILTER (WHERE balance > 0)` (active-holder semantics, matching the Stellar ecosystem convention used by StellarExpert / Stellarchain.io) and `total_supply = SUM(balance)` from `account_balances_current`. **MVP scope** — Stellar protocol stores no `AssetEntry` / `AssetSupplyEntry` on-chain, so supply is always derived. Horizon `/assets` aggregates 4 sources (trustlines + claimable_balances + LP reserves + SAC contract holdings); MVP aggregates only trustlines. Drift on heavily-used DeFi assets can be material (~20-50% under-count vs Horizon for USDC w/ heavy Soroswap + SAC use). Full Horizon parity tracked under task 0194 Future Work. Recompute (rather than per-trustline delta) avoids ON-CONFLICT-vs-INSERT introspection on the upsert path; the affected-set is bounded per ledger so the cost stays small. Implementation owned by task 0194 §1b (total_supply) + §1c (holder_count, supersedes blocked task 0135). **Type-3 (bespoke Soroban tokens) — task 0331:** these have no trustlines, so `total_supply` / `holder_count` derive from the unified `balances` table (per-holder `ContractData` `Balance(Address)` ledger STATE — NOT an event-fold, which under-counts vault / rebasing / non-SEP-41-event tokens), aggregated by `balance_aggregates` (`sum(amount)` / `countIf(amount > 0)`). `total_supply = sum(amount)` is the SOLE supply source (Option A — no per-token `TotalSupply` key read; a mint always credits a holder balance, and contract treasuries are summed because holders include `C…`, so the sum equals real supply). RAW `Int128` (scale by `decimals`), distinct from the classic pre-scaled `Decimal128(7)`. SAC contract-held balances (Horizon source #4 for classic parity) ride the same `balances` table — tracked under task 0210 Phase 3 / 0331 follow-up D2.
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
- **`accounts_recent` (ClickHouse read-model, task 0385):** the account-list browse
  (`GET /v1/accounts`) sorts by `last_seen_ledger`, but the CH `accounts` table must
  be `ORDER BY account_id` — that is the `ReplacingMergeTree` dedup key, and
  `last_seen_ledger` mutates so it cannot sit in the sort key. A projection on the
  RMT is refused by ClickHouse 26.3 (task 0353, Code 344), so the last_seen ordering
  lives in a separate plain-`MergeTree` table filled by a refreshable MV (full
  recompute + atomic EXCHANGE → reads need no `FINAL`; mirrors `balance_aggregates_mv`,
  §4.17). `accounts::fetch_list` read-in-order SEEKs it (~page rows) instead of the
  old `accounts FINAL` whole-dimension scan+sort (~24M). Freshness = the refresh
  interval — a shared, server-side origin, ≤interval-stale, fine for a browse list.

  ```sql
  CREATE TABLE accounts_recent (
      id                Int64,
      account_id        String,
      last_seen_ledger  Int64,
      first_seen_ledger Int64,
      home_domain       LowCardinality(Nullable(String))
  ) ENGINE = MergeTree
  ORDER BY (last_seen_ledger, id);

  CREATE MATERIALIZED VIEW accounts_recent_mv
  REFRESH EVERY 2 MINUTE
  TO accounts_recent AS
  SELECT id, account_id, last_seen_ledger, first_seen_ledger, home_domain
  FROM accounts FINAL;
  ```

### 4.12 NFTs

```sql
CREATE TABLE nfts (
    id                   SERIAL       PRIMARY KEY,
    contract_id          BIGINT       NOT NULL REFERENCES soroban_contracts(id),  -- ADR 0030
    token_id             VARCHAR(256) NOT NULL,
    collection_name      VARCHAR(256),                                            -- task 0195 §2d (Lambda 2)
    name                 VARCHAR(256),                                            -- task 0195 §2d (Lambda 2)
    media_url            TEXT,                                                    -- task 0195 §2d (Lambda 2)
    -- (`metadata JSONB` dropped per ADR 0043 / task 0195 §2d — detail-only,
    --  served at request time via `runtime_enrichment::nft_token_uri`)
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

### 4.13.1 NFT Quarantine — `nfts_pending` + `nft_ownership_pending` (task 0217)

```sql
CREATE TABLE nfts_pending (
    contract_id           BIGINT       NOT NULL,  -- no FK to soroban_contracts
    token_id              VARCHAR(256) NOT NULL,
    collection_name       VARCHAR(256),
    name                  VARCHAR(256),
    media_url             TEXT,
    minted_at_ledger      BIGINT,
    current_owner_id      BIGINT,                  -- no FK to accounts
    current_owner_ledger  BIGINT,
    PRIMARY KEY (contract_id, token_id)
);
CREATE INDEX idx_nfts_pending_contract ON nfts_pending(contract_id);

CREATE TABLE nft_ownership_pending (
    contract_id      BIGINT       NOT NULL,
    token_id         VARCHAR(256) NOT NULL,
    transaction_id   BIGINT       NOT NULL,         -- no FK
    owner_id         BIGINT,                         -- no FK
    event_type       SMALLINT     NOT NULL,         -- ADR 0031 NftEventType
    ledger_sequence  BIGINT       NOT NULL,
    event_order      SMALLINT     NOT NULL,
    created_at       TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (contract_id, token_id, created_at, ledger_sequence, event_order),
    CONSTRAINT ck_nft_own_pending_event_type_range CHECK (event_type BETWEEN 0 AND 15)
);
CREATE INDEX idx_nft_ownership_pending_contract ON nft_ownership_pending(contract_id);
```

Purpose:

- isolate NFT-candidate rows whose contract has not yet been definitively
  classified (verdict `Other` or NULL — no usable WASM observed in any indexed
  ledger so far). The API-facing hot tables (`nfts` / `nft_ownership`) stay
  clean by design.

Persist routing (task 0217 Phase B, see
[`crates/indexer/src/handler/persist/write.rs`](../../../crates/indexer/src/handler/persist/write.rs)):

| Classifier verdict   | Target tables                            |
| -------------------- | ---------------------------------------- |
| `Nft` (=2)           | `nfts` + `nft_ownership` (hot)           |
| `Fungible` / `Token` | _none_ (filtered out)                    |
| `Other` (=1) / NULL  | `nfts_pending` + `nft_ownership_pending` |

Promotion is wired through the existing `reclassify_contracts_from_wasm`
UPDATE path (originally task 0118 Phase 2). When a contract's verdict flips
`Other → Nft`, the quarantine rows move into the hot tables in the same
transaction; on `Other → Fungible`/`Token` they are dropped without an
intermediate hot insert. API endpoints (`/v1/nfts*`) never read the
`_pending` tables.

Design notes:

- **No FKs to `soroban_contracts` / `accounts` / `transactions` / `nfts`** —
  rows arrive transient and the FK lookup churn has no read-side payoff
  (the only read is the per-`contract_id` promotion lookup).
- **No partitioning** — pending is transient; the by-`created_at` range
  pattern that drives `nft_ownership`'s partitioning has no read-side
  payoff here.
- **Minimal indexing** — single `(contract_id)` btree on each table.
- Natural-key PKs (`(contract_id, token_id)` and
  `(contract_id, token_id, created_at, ledger_sequence, event_order)`)
  enable column-projection promotion via `INSERT INTO nfts SELECT …`
  without needing to resolve a SERIAL `nfts.id`.

Operational lifecycle: see
[`docs/runbooks/0217_nfts_pending_migration_and_drain.md`](../../runbooks/0217_nfts_pending_migration_and_drain.md)
for the one-shot migration of existing `Other`/NULL hot rows into the
quarantine and the post-backfill drain procedure. The decision record
for the quarantine pattern (alternatives considered, design rationale,
consequences) lives in
[ADR 0046](../../../lore/2-adrs/0046_classifier-quarantine-tables-nfts-pending.md).

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
- **Sentinel placeholder rows** ([ADR 0042](../../../lore/2-adrs/0041_lp-positions-orphan-handling-state-filter-and-sentinel-pool.md)):
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
  **API filter (task 0193):** every pool-surfacing endpoint excludes sentinels
  at two layers. (1) The handler-level `pool_exists()` gate carries
  `created_at_ledger > 0` so per-pool look-ups of a sentinel return 404 before
  the per-endpoint query runs. (2) Each of the five canonical LP SQL queries
  (`18_*.sql`, `19_*.sql`, `20_*.sql`, `21_*.sql`, `23_*.sql`; `22_*.sql` is
  `get_search` and unrelated) carries its own sentinel predicate: `18` / `19`
  filter `lp.created_at_ledger > 0` inline (they read `liquidity_pools`
  directly); `20` / `21` / `23` add an `EXISTS (SELECT 1 FROM liquidity_pools
… WHERE created_at_ledger > 0)` guard. The redundancy is defense-in-depth —
  a future caller bypassing the handler still gets an empty result.

### 4.15 Liquidity Pool Snapshots

```sql
CREATE TABLE liquidity_pool_snapshots (
    pool_id         FixedString(32),
    ledger_sequence Int64,
    reserve_a       Decimal128(7),
    reserve_b       Decimal128(7),
    total_shares    Decimal128(7),
    gross_volume_a  Nullable(Decimal128(7))   -- asset-A-unit gross trade volume (task 0261/0266)
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (pool_id, ledger_sequence);
```

Purpose:

- persist time-series pool state for chart endpoints and recent-trend analysis
- decouple pool-chart reads from live recomputation over raw transaction history

Design notes:

- snapshot rows are append-only and written in ledger order
- composite `(id, created_at)` PK is required by the partitioning key rule;
  `pool_id` is `BYTEA(32)` (ADR 0024) with the deferred FK back to `liquidity_pools`
- reserves are typed `NUMERIC(28,7)` columns (not JSONB), uniform with the rest of
  the schema's balance / amount handling
- `tvl`, `volume` and `fee_revenue` are **removed** (task 0374 distillation;
  the production `ALTER ... DROP COLUMN` rides the 0374 deploy — see
  `docs/deployment.md`, where the order is load-bearing for the 0.15 driver).
  They were written as NULL since task 0199, which landed the analytics as
  **compute-at-read** instead ([ADR 0053](../../../lore/2-adrs/0053_fast-change-offchain-compute-at-read.md)):
  the API multiplies the on-chain quantities in these rows (`reserve_a/b`, and
  `gross_volume_a`) by USD closes read at query time from the prices service's
  in-cluster `prices.*` views. The earlier plan — a Lambda 2 write-back per
  ADR 0043 — was rejected because `liquidity_pool_snapshots` is a
  `ReplacingMergeTree` with no version column, so a per-row write-back is a
  racy read-modify-write that a later plain insert can silently erase. An
  eventual USD materialization gets its own table with its own writer; these
  columns are not its home.
- `gross_volume_a` (asset-A-unit gross trade volume per `(pool, ledger)`, from
  PathPayment claim atoms) IS populated — live since task 0261 and backfilled
  by 0266 — and is the on-chain input the read-time USD `volume` multiplies.
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

ClickHouse has no B-tree secondary indexes; the PG-era index families this
section used to describe (GIN, trigram, partial-unique, `text_pattern_ops`
prefix btrees) have no CH equivalent and were retired with the store. The
CH read-acceleration model has four layers, all declared in
`crates/db-clickhouse/schema/init.sql`:

1. **The sort key is the index.** Each table's `ORDER BY` defines its sparse
   primary index; every seek path is designed against it (per-table sort keys
   are in the DDL and in §8.0). Uniqueness is not enforced by indexes at all —
   `ReplacingMergeTree` deduplicates by sort key at merge/read time, which is
   why every read must dedup (`FINAL`, `GROUP BY` + `max()`, or
   `LIMIT 1 BY`) — see the RMT rules in
   [`clickhouse-pilot.md §5`](./clickhouse-pilot.md).
2. **Skip indexes** bound the paths the sort key cannot serve. The deployed
   inventory (each entry in `init.sql` carries the measurement and — per the
   0400 lesson — its named consumer; keep prod and file in sync both ways):

   | Table                    | Index                | Type                  | Serves                                                                                                                         |
   | ------------------------ | -------------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
   | `ledgers`                | `closed_at_mm`       | `minmax`              | time-window → ledger-range resolution (LP chart); measured 77.9M → 26.3M read_rows/req on the 2026-07-17 load test (task 0357) |
   | `accounts`               | `idx_acc_id`         | `bloom_filter(0.001)` | surrogate-id → StrKey seeks: tx-list/search in `crates/api` + SEP-1 issuer resolve in the enrichment worker (tasks 0290, 0397) |
   | `soroban_contracts`      | `idx_sc_id`          | `bloom_filter(0.001)` | tx-detail surrogate-id → `contract_id` resolve (task 0344)                                                                     |
   | `transactions`           | `idx_tx_hash_bloom`  | `bloom_filter(0.01)`  | point lookup by hash within the partition named by `transaction_hash_index`                                                    |
   | `operations_appearances` | `idx_oa_pool_ids`    | `bloom_filter(0.001)` | sparse-pool regime of the pool-transactions scan (task 0365)                                                                   |
   | `operations_appearances` | `idx_oa_contract_id` | `bloom_filter(0.001)` | sparse-contract regime of the contract-filtered tx list (task 0333; the 2026-06-29 quota blowout)                              |

3. **A `Dictionary`** (`transaction_hash_dict`) serves hot
   hash → `(ledger_sequence, application_order)` point lookups without
   touching the base table (§4e in `clickhouse-pilot.md`, task 0204). Its
   `ComplexKeyCache` layout reports `element_count` as _currently cached_
   entries, not source size — 1 on a cold cache is healthy, not broken.
4. **Projections: none, by constraint.** ClickHouse 26.3 refuses projections
   on `ReplacingMergeTree` (`Code 344`, measured in task 0353); anything a
   projection would have served is done with an MV or a skip index instead.

Two conventions guard this inventory (both are 0400 scar tissue):

- **Every index names its consumer** in an `init.sql` comment. An index whose
  consumer disappears gets dropped on both sides — `idx_oa_asset_issuer_id`
  (consumer moved, task 0359) and `idx_oaa_transaction_id` (declared for a
  withdrawn read path, dropped from prod AND the file 2026-08-06) are the
  precedents.
- **Drift runs in both directions.** Prod-only indexes (`closed_at_mm` lived
  only on the box for a while) and file-only indexes are both defects; online
  `ALTER … ADD INDEX + MATERIALIZE INDEX` must land in `init.sql` in the same
  change.

Column-type choices still carry the indexing economics: `FixedString(32)`
hashes ([ADR 0024](../../../lore/2-adrs/0024_hashes-bytea-binary-storage.md),
translated per `clickhouse-pilot.md §6`) and narrow enum columns
([ADR 0031](../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md))
keep the sparse index and skip-index granule sets small at mainnet-year
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

On ClickHouse, partitioning is declared in the table DDL as `PARTITION BY
intDiv(sequence, 500000)` (500k-ledger blocks) and ClickHouse creates the
parts automatically on insert — there is no provisioning step. (The PG-era
partition-management Lambda `crates/db-partition-mgmt`, which pre-created
monthly `<table>_y{YYYY}m{MM}` partitions, was removed with the PG→CH
cutover — task 0241.) Partitioning keeps retention, maintenance, and
ledger-sliced reads practical on the high-write tables.

### 6.3 Retention Model

The current retention statement is conservative:

- ledger and transaction history are kept indefinitely
- partitioned time-series tables may be pruned only if storage constraints require it
- ClickHouse creates partitions automatically on insert; pruning is an operator
  `ALTER TABLE … DROP PARTITION`, never ad-hoc application code

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

#### Canonical query references

Each `/v1/*` list / detail endpoint has a canonical SQL projection committed
in this repo:

- [`endpoint-queries-clickhouse/`](./endpoint-queries-clickhouse/) — **ClickHouse**
  canonical source of truth for the live API handlers in
  [`crates/api/`](../../../crates/api)
  ([ADR 0044](../../../lore/2-adrs/0044_clickhouse-pilot-parallel-store.md),
  tasks 0204 / 0206 / 0207). Every endpoint enumerated in
  [`backend-overview.md §6.2 Endpoint Inventory`](../backend/backend-overview.md#62-endpoint-inventory)
  maps 1:1 to a `NN_*.sql` file here.
  Field-allocation per [ADR 0043](../../../lore/2-adrs/0043_field-allocation-rule-list-vs-detail.md);
  list-endpoint completeness verified by audit task 0197
  (see [`docs/audits/2026-05-13-0197-step0/2026-05-13-list-endpoint-completeness.md`](../../audits/2026-05-13-0197-step0/2026-05-13-list-endpoint-completeness.md)).

The retired PostgreSQL reference set (`endpoint-queries/`) was removed with the
PG backend (task 0244).

When editing a `/v1/*` endpoint behaviour, update the CH canonical SQL in the
same PR.

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

### 8.0 ClickHouse (sole production store)

ClickHouse is the sole production store (task 0244 — Postgres retired). It began
as a parallel pilot per
[ADR 0044](../../../lore/2-adrs/0044_clickhouse-pilot-parallel-store.md)
(implementation in
[task 0204](../../../lore/1-tasks/active/0204_FEATURE_clickhouse-pilot-crate-docker-schema/README.md)),
then took over ingestion and all API reads in the hard cutover
[task 0241](../../../lore/1-tasks/archive/0241_FEATURE_indexer-hard-swap-pg-to-ch-and-cutover-runbook.md).
It carries the table-by-table logical shape described above, with five
deliberate divergences from the former PG schema (full-content `soroban_events`
replacing `soroban_events_appearances`, `created_at` dropped from every table
except `ledgers`, `nfts.metadata` dropped, `_sqlx_migrations` replaced by an
idempotent `init.sql`, `transaction_hash_index` also exposed as a `Dictionary`
for hot point lookups).

The store lives in `crates/db-clickhouse/` (schema `init.sql`) and runs as the
`clickhouse` service in `docker-compose.yml`. The full physical schema reference,
including the type-translation table and divergence rationale, lives in
[`clickhouse-pilot.md`](./clickhouse-pilot.md).

## 8. Evolution Rules and Delivery Notes

### 8.1 Schema Evolution Rules

Any future schema change should preserve the same general discipline:

- add new tables or columns only when tied to a documented explorer or ingestion need
- avoid replacing explicit relational structure with oversized generic JSON blobs
- keep public lookup keys stable where routes or API contracts depend on them
- update the general overview first if the conceptual schema changes materially

### 8.2 Current Workspace State

The repository provides concrete DDL for every table in §4 in the ClickHouse schema
`crates/db-clickhouse/schema/init.sql` — a single idempotent `CREATE … IF NOT EXISTS`
script applied by `db-clickhouse-init` (there is no ordered migration sequence).
Runtime persistence lives in the indexer, which stages and writes via the
`crates/db-clickhouse/src/persist/` pipeline; the post-surrogate schema rationale is
[ADR 0027](../../../lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md).

This document is the detailed schema reference; the narrative
[`technical-design-general-overview.md`](../technical-design-general-overview.md)
remains the primary source of truth for cross-component behavior, and the
`lore/2-adrs/**` trail is authoritative for the "why" behind any individual schema
decision. Per
[ADR 0032](../../../lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md),
any future schema change must also update this file in the same PR.
