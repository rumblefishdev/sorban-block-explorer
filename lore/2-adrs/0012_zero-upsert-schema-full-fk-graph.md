---
id: '0012'
title: 'Zero-upsert DB schema with full FK graph, activity projections, and complete index strategy'
status: proposed
deciders: [fmazur]
related_tasks: []
related_adrs: ['0004', '0005', '0011']
tags:
  [
    database,
    schema,
    architecture,
    reconstructability,
    integrity,
    indexes,
    performance,
  ]
links: []
history:
  - date: 2026-04-17
    status: proposed
    who: fmazur
    note: 'ADR created — refinement of ADR 0011 based on per-block reconstructability requirement and full referential integrity'
  - date: 2026-04-17
    status: proposed
    who: fmazur
    note: 'Added S3 file structure spec with ledger_metadata header and nft_metadata array. Removed transactions.memo/memo_type/result_code and nfts.metadata from DB (now S3-only). Added ledgers.protocol_version.'
  - date: 2026-04-17
    status: proposed
    who: fmazur
    note: 'Removed 14 FK constraints from ledger_sequence / *_at_ledger columns (dimensional modeling). ledgers is now treated as a dimension table, not entity hub. FK count reduced from 39 to 25. Parallel backfill pipeline simplified — ledgers row can be written at any point.'
  - date: 2026-04-17
    status: proposed
    who: fmazur
    note: 'Added activity projection tables (account_activity, token_activity, nft_current_ownership, token_current_supply, liquidity_pool_current, contract_stats_daily, search_index). Endpoints that cannot be indexed against core event-log tables now have dedicated denormalized read models. Added full indexing strategy: backfill-time minimum + post-backfill CONCURRENTLY build. Preserves HOT on progressive-COALESCE tables. BRIN on monotonic ledger/time columns, partial indexes on nullable FKs, pg_trgm for base32 addresses.'
---

# ADR 0012: Zero-upsert DB schema with full FK graph, activity projections, and complete index strategy

**Related:**

- [ADR 0004: Rust-only XDR parsing](0004_rust-only-xdr-parsing.md)
- [ADR 0005: Rust-only backend API](0005_rust-only-backend-api.md)
- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)

---

## Context

ADR 0011 defined a lightweight DB schema with heavy parsed data offloaded to S3. It relied
on **watermark upserts** on several tables (`accounts`, `nfts.current_owner`, `soroban_contracts`)
and intentionally **omitted foreign keys** between tables that could be populated out of
order by parallel backfill workers.

Three requirements surfaced that make that design insufficient:

1. **Per-block reconstructability.** Users want to answer "what was the state of the chain
   at ledger X?" for accounts, balances, NFT ownership, token supply, and pool state. The
   upsert pattern overwrites mutable state, making past states unrecoverable without full
   chain re-indexing.

2. **Referential integrity.** Missing FKs mean the DB cannot enforce consistency — orphan
   rows, dangling references, and typos in IDs pass silently. At mainnet scale this is a
   real operational risk: bugs in the parser can corrupt data, and nothing in the DB layer
   will flag it.

3. **Endpoint serviceability at scale.** ~40% of the public API surface asks per-entity
   activity questions ("all transactions involving account X", "all transactions touching
   token Y", "NFTs currently owned by X"). Event-log tables (`transactions`, `operations`,
   `soroban_events`, `nft_ownership`) do not expose these joins efficiently: account is
   referenced from 5+ places, token activity spans `operations` (classic) and
   `soroban_events` (Soroban), current-owner-per-NFT needs DISTINCT ON over billions of
   rows. No index on the event log fixes this — the missing piece is a denormalized
   activity projection, standard in block-explorer designs (Etherscan `tx_by_address`,
   `token_tx`, etc.).

This ADR specifies a refined schema that satisfies all three requirements:

- **Zero-upsert design** for all historically meaningful data — every mutable field lives
  in a dedicated insert-only history table, every "current state" is a
  `ORDER BY ledger_sequence DESC LIMIT 1` query.
- **Full foreign key graph** — every logical parent-child relationship enforced at the DB
  level, with `ON DELETE RESTRICT` across the board.
- **Identity-first ingestion pattern** — enables parallel backfill without sacrificing FKs.
- **Activity projection tables** — denormalized per-entity activity feeds populated at
  persist time alongside the event log. Rebuildable from the zero-upsert source of truth.
- **Complete indexing strategy** — backfill-time minimum (implicit PK/UNIQUE only) and
  post-backfill `CREATE INDEX CONCURRENTLY` build, preserving HOT updates on
  progressive-COALESCE tables and keeping write amplification near 2× during backfill.

The S3 offload strategy from ADR 0011 is preserved: heavy parsed JSON (XDRs, operation
details, event payloads, WASM metadata) lives on S3, DB keeps only lightweight index and
filter columns. The present ADR supersedes ADR 0011's _table-level_ decisions and adopts
its _S3-related_ decisions unchanged.

---

## Decision

### Design principles

1. **Immutable identity tables** (insert-once): `accounts`, `nfts`, `tokens`,
   `liquidity_pools`, `soroban_contracts`, `wasm_interface_metadata`, `ledgers`,
   `transactions`. Pure tuple of identity; any `INSERT` that violates uniqueness is a
   no-op (`ON CONFLICT DO NOTHING`).

2. **Insert-only history tables** for all mutable state:

   - `account_balances` — balance per asset per ledger
   - `account_home_domain_changes` — home_domain changes
   - `nft_ownership` — NFT owner per ledger event
   - `token_supply_snapshots` — total supply and holder count per ledger
   - `liquidity_pool_snapshots` — pool state per ledger

3. **Fact tables** (append-only): `operations`, `soroban_events`, `soroban_invocations`.
   These are immutable per chain, not "history of something" but "events that happened".

4. **Activity projection tables** (denormalized read models):
   `account_activity`, `token_activity`, `nft_current_ownership`,
   `token_current_supply`, `liquidity_pool_current`, `contract_stats_daily`,
   `search_index`. These are derived from the event log at persist time; they are
   denormalized exceptions to zero-upsert (current-state projections use upsert) but
   remain rebuildable from the source of truth at any time.

5. **Derivation** where existing data suffices: `accounts.sequence_number` derived from
   `transactions.source_post_sequence_number`, `accounts.last_seen_ledger` from
   `MAX(transactions.ledger_sequence)`.

6. **Full FK graph.** Every logical relationship modeled as FK. `ON DELETE RESTRICT`
   everywhere — on-chain data is immutable, no DELETE path in normal operation.

7. **No CHECK constraints on VARCHAR values.** Type and length enforced at DB layer,
   value consistency (enumeration) enforced at parser/API layer. Keeps schema flexible
   to protocol evolution (new operation types, new event categories) without migration.

### S3 offload

Principles inherited from ADR 0011; JSON structure restated here with additions
(`ledger_metadata` header and `nft_metadata` array) required by this ADR.

- One file per ledger: `parsed_ledger_{sequence}.json`
- Parse phase produces full JSON, persist phase writes to S3 + DB in parallel
- DB bridge columns locating the correct file: `ledger_sequence` (most tables),
  `deployed_at_ledger` (`soroban_contracts`), `uploaded_at_ledger`
  (`wasm_interface_metadata`), `metadata_ledger` (`tokens`), `minted_at_ledger`
  (`nfts`)
- List endpoints: DB only. Detail endpoints: DB + at most 1 S3 fetch.

**File structure:**

```json
{
  "ledger_sequence": 12345,
  "ledger_metadata": {
    "hash": "abc123...",
    "closed_at": "2026-04-17T12:34:56Z",
    "protocol_version": 21,
    "tx_count": 42,
    "op_count": 89,
    "base_fee": 100
  },
  "transactions": [
    {
      "hash": "def456...",
      "source_account": "GABC...",
      "memo_type": "text",
      "memo": "payment for services",
      "result_code": "tx_success",
      "signatures": [
        {"public_key": "GABC...", "signature": "deadbeef..."}
      ],
      "envelope_xdr": "AAAA...",
      "result_xdr": "AAAA...",
      "result_meta_xdr": "AAAA...",
      "operation_tree": [...],
      "operations": [
        {"index": 0, "details": {...}}
      ],
      "events": [
        {"index": 0, "topics": [...], "data": {...}}
      ],
      "invocations": [
        {"index": 0, "function_args": [...], "return_value": {...}}
      ]
    }
  ],
  "wasm_uploads": [
    {
      "wasm_hash": "fed789...",
      "functions": [{"name": "swap", "inputs": [...], "outputs": [...]}],
      "wasm_byte_len": 45230,
      "name": "Soroswap Router"
    }
  ],
  "contract_metadata": [
    {"contract_id": "CABC...", "metadata": {...}}
  ],
  "token_metadata": [
    {"token_id": 42, "metadata": {...}}
  ],
  "nft_metadata": [
    {"contract_id": "CABC...", "token_id": "1234", "metadata": {...}}
  ]
}
```

**Notes on new sections vs ADR 0011:**

- `ledger_metadata` — enables `GET /ledgers/:sequence` to be served from a single S3
  fetch without any DB query. Previously the endpoint needed DB lookup for
  ledger-level metadata (hash, closed_at, protocol_version, etc.). Parser already
  has this data in parse phase — no additional work.
- `nft_metadata` — NFT metadata JSONB moves out of the `nfts` table to S3,
  maintaining consistency with `contract_metadata` and `token_metadata`. Indexed
  by `(contract_id, token_id)` pair to allow the API to extract a specific NFT's
  metadata from the ledger file.
- `transactions[].memo`, `memo_type`, `result_code` — these columns move out of
  the `transactions` DB table to here. They are displayed only in transaction
  detail view (which already fetches this file for `operation_tree`), so no
  additional S3 round-trip is required.

### Column sizing conventions (inherited from ADR 0011)

- **Account addresses:** `VARCHAR(69)` (muxed M-addresses per SEP-0023)
- **Contract addresses:** `VARCHAR(56)` (C-prefixed, never muxed)
- **Token amounts:** `NUMERIC(39,0)` (raw i128 integers per SEP-0041)
- **Pool metrics with undefined precision:** bare `NUMERIC`
- **Hash values:** `VARCHAR(64)` (hex-encoded)

### Parallel backfill strategy

Workers process disjoint ledger ranges in parallel. To satisfy FK constraints, each worker
uses the **identity-first pattern**:

```
Per-ledger persist order:
  1. accounts           -- INSERT ON CONFLICT DO NOTHING for every seen account
  2. soroban_contracts  -- INSERT ON CONFLICT DO NOTHING (stub for referenced contracts)
  3. tokens, nfts, liquidity_pools, wasm_interface_metadata  -- identity rows
  4. transactions       -- now source_account FK resolves
  5. operations, soroban_events, soroban_invocations  -- reference transactions
  6. account_balances, account_home_domain_changes
  7. nft_ownership
  8. token_supply_snapshots, liquidity_pool_snapshots
  9. ledgers            -- row for this ledger can be written at any time (no FK
                        -- constraints depend on it); parallel write from other
                        -- workers is also fine (ON CONFLICT DO NOTHING)
 10. activity projections -- account_activity, token_activity rows emitted from
                        -- accumulated per-tx context
 11. *_current projections -- upsert nft_current_ownership, token_current_supply,
                        -- liquidity_pool_current from the freshly inserted history rows
                        -- (only when incoming ledger_sequence > stored watermark)
 12. search_index       -- upsert identity-level search rows
```

Since `ledger_sequence` / `*_at_ledger` columns are not FK-enforced (see "Why
ledger_sequence is not a FK"), no child table has to wait for the `ledgers` row to be
inserted. Workers can write their entire per-ledger payload independently, and the
`ledgers` identity row can arrive at any point during or after that write.

Identity tables use `INSERT ... ON CONFLICT (pk) DO NOTHING` — any worker can create the
stub row, later workers processing richer data (e.g. WASM upload producing full
`soroban_contracts.name`) upsert via COALESCE to fill NULLs. **COALESCE only fills NULLs,
never overwrites known values** — this preserves the zero-overwrite invariant.

**Cross-worker ordering example:**

1. Worker A (ledger 50000) sees NFT transfer for token X. NFT X's mint happened at ledger
   10000, but Worker B hasn't processed that yet.
2. Worker A: `INSERT INTO nfts (contract_id, token_id) VALUES (...) ON CONFLICT DO NOTHING`
   — creates stub with NULL name/media_url/collection_name.
3. Worker A: `INSERT INTO nft_ownership (nft_id, ...)` — FK resolves.
4. Worker B (ledger 10000) later: processes mint, COALESCE fills `name`, `media_url`,
   `collection_name`, `minted_at_ledger`. Full metadata JSONB lands in the S3 file
   for ledger 10000 under `nft_metadata[]`.
5. End state: single DB row with complete display data + S3 payload, no duplicate, no FK violation.

### Core tables — schema

#### 1. `ledgers`

Top-level entity. No FK to anything else.

```sql
ledgers (
  sequence          BIGINT PRIMARY KEY,
  hash              VARCHAR(64) NOT NULL UNIQUE,
  tx_count          INTEGER NOT NULL,
  op_count          INTEGER NOT NULL,
  closed_at         TIMESTAMPTZ NOT NULL,
  protocol_version  INTEGER NOT NULL,
  base_fee          BIGINT
)
-- Insert-only. ON CONFLICT (sequence) DO NOTHING.
-- Columns also duplicated in the S3 file's `ledger_metadata` header, so that
-- GET /ledgers/:sequence can be served from a single S3 fetch without any DB query.
-- DB retains the row for list endpoint (GET /ledgers) and cross-ledger queries.
-- NOTE: `ledgers` is a dimension table, not an entity hub. Other tables store
-- `ledger_sequence` / `*_at_ledger` columns WITHOUT FK to this table (see
-- "Why ledger_sequence is not a FK" section for rationale).
```

#### 2. `transactions`

Lightweight row. All heavy and detail-only fields on S3.

```sql
transactions (
  id                          BIGSERIAL PRIMARY KEY,
  hash                        VARCHAR(64) NOT NULL UNIQUE,
  ledger_sequence             BIGINT NOT NULL,
  source_account              VARCHAR(69) NOT NULL
                              REFERENCES accounts(account_id) ON DELETE RESTRICT,
  source_post_sequence_number BIGINT,
  fee_charged                 BIGINT NOT NULL,
  successful                  BOOLEAN NOT NULL,
  created_at                  TIMESTAMPTZ NOT NULL,
  parse_error                 BOOLEAN
)
-- source_post_sequence_number: sequence_number of source_account AFTER this tx,
-- extracted from result meta ledger entry change. Enables deriving account state
-- at any ledger. Correctly handles BumpSequence (stores post-state, not prev+1).
```

**Offloaded to S3:** `envelope_xdr`, `result_xdr`, `result_meta_xdr`, `operation_tree`,
`signatures[]`, `memo`, `memo_type`, `result_code`.

Rationale for removing `memo`, `memo_type`, `result_code` from DB: none are used for
filtering, sorting, or list-view display. They appear only in transaction detail view,
which already fetches the S3 ledger file for `operation_tree`. Saves ~18 GB at mainnet
scale with zero added S3 round-trips.

#### 3. `operations`

Fact table. Partitioned by `created_at` (see "Why operations partitions by created_at"
for rationale — aligned with events/invocations for uniform temporal pruning).

```sql
operations (
  id                BIGSERIAL,
  transaction_id    BIGINT NOT NULL
                    REFERENCES transactions(id) ON DELETE RESTRICT,
  application_order SMALLINT NOT NULL,
  source_account    VARCHAR(69) NOT NULL
                    REFERENCES accounts(account_id) ON DELETE RESTRICT,
  type              VARCHAR(50) NOT NULL,
  destination       VARCHAR(69)
                    REFERENCES accounts(account_id) ON DELETE RESTRICT,
  contract_id       VARCHAR(56)
                    REFERENCES soroban_contracts(contract_id) ON DELETE RESTRICT,
  function_name     VARCHAR(100),
  asset_code        VARCHAR(12),
  asset_issuer      VARCHAR(69),
  pool_id           VARCHAR(64)
                    REFERENCES liquidity_pools(pool_id) ON DELETE RESTRICT,
  created_at        TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (id, created_at),
  UNIQUE (transaction_id, application_order, created_at)
) PARTITION BY RANGE (created_at);
-- Dedup: ON CONFLICT (transaction_id, application_order, created_at) DO NOTHING
-- Extracted filter columns (destination, contract_id, function_name, asset_code,
-- asset_issuer, pool_id) — full details JSONB on S3.
```

#### 4. `accounts`

Pure immutable identity. Mutable state in dedicated history tables.

```sql
accounts (
  account_id         VARCHAR(69) PRIMARY KEY,
  first_seen_ledger  BIGINT NOT NULL
)
-- Insert-once. ON CONFLICT (account_id) DO NOTHING.
-- sequence_number   → derive from transactions.source_post_sequence_number
-- last_seen_ledger  → derive from MAX(transactions.ledger_sequence) WHERE source_account=?
-- home_domain       → account_home_domain_changes
-- balances          → account_balances
```

#### 5. `account_balances`

Insert-only balance history per asset per ledger.

```sql
account_balances (
  id               BIGSERIAL PRIMARY KEY,
  account_id       VARCHAR(69) NOT NULL
                   REFERENCES accounts(account_id) ON DELETE RESTRICT,
  ledger_sequence  BIGINT NOT NULL,
  asset_type       VARCHAR(20) NOT NULL,
  asset_code       VARCHAR(12) NOT NULL DEFAULT '',
  issuer           VARCHAR(69) NOT NULL DEFAULT '',
  event_order      SMALLINT NOT NULL DEFAULT 0,
  balance          NUMERIC(39,0) NOT NULL,
  UNIQUE (account_id, ledger_sequence, asset_type, asset_code, issuer, event_order)
)
-- Insert-only. Native XLM: asset_code='', issuer='' (empty strings, not NULL).
-- event_order disambiguates multiple balance changes in the same ledger
-- (e.g. path payment hops, Soroban batch transfer).
-- Dedup: ON CONFLICT (...) DO NOTHING.
```

#### 6. `account_home_domain_changes`

Insert-only history of home_domain changes. Rare events (~once per account lifetime).

```sql
account_home_domain_changes (
  id               BIGSERIAL PRIMARY KEY,
  account_id       VARCHAR(69) NOT NULL
                   REFERENCES accounts(account_id) ON DELETE RESTRICT,
  ledger_sequence  BIGINT NOT NULL,
  home_domain      VARCHAR(256),
  UNIQUE (account_id, ledger_sequence)
)
-- Emitted when SET_OPTIONS changes home_domain. NULL home_domain = cleared.
-- Dedup: ON CONFLICT (account_id, ledger_sequence) DO NOTHING.
```

#### 7. `soroban_contracts`

Contract identity + classification. Full metadata on S3.

```sql
soroban_contracts (
  contract_id        VARCHAR(56) PRIMARY KEY,
  wasm_hash          VARCHAR(64)
                     REFERENCES wasm_interface_metadata(wasm_hash) ON DELETE RESTRICT,
  deployer_account   VARCHAR(69)
                     REFERENCES accounts(account_id) ON DELETE RESTRICT,
  deployed_at_ledger BIGINT,
  contract_type      VARCHAR(50),
  is_sac             BOOLEAN NOT NULL DEFAULT FALSE,
  name               VARCHAR(256),
  search_vector      TSVECTOR GENERATED ALWAYS AS
                     (to_tsvector('simple', coalesce(name, ''))) STORED
)
-- Progressive fill via COALESCE. Stub may be created by a worker that sees a
-- reference to the contract before the deploy ledger is processed; later
-- workers fill in wasm_hash/deployer_account/deployed_at_ledger/name/contract_type.
-- COALESCE never overwrites non-NULL values — preserves zero-overwrite invariant.
```

#### 8. `soroban_events`

Fact table. Topics and data on S3. Partitioned by `created_at`.

```sql
soroban_events (
  id               BIGSERIAL,
  transaction_id   BIGINT NOT NULL
                   REFERENCES transactions(id) ON DELETE RESTRICT,
  contract_id      VARCHAR(56)
                   REFERENCES soroban_contracts(contract_id) ON DELETE RESTRICT,
  event_type       VARCHAR(20) NOT NULL,
  topic0           VARCHAR(100),
  event_index      SMALLINT NOT NULL DEFAULT 0,
  ledger_sequence  BIGINT NOT NULL,
  created_at       TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (id, created_at),
  UNIQUE (transaction_id, event_index, created_at)
) PARTITION BY RANGE (created_at);
-- topic0: first topic extracted for filtering (event name).
-- Dedup: ON CONFLICT (transaction_id, event_index, created_at) DO NOTHING.
```

#### 9. `soroban_invocations`

Fact table. function_args and return_value on S3. Partitioned by `created_at`.

```sql
soroban_invocations (
  id                BIGSERIAL,
  transaction_id    BIGINT NOT NULL
                    REFERENCES transactions(id) ON DELETE RESTRICT,
  contract_id       VARCHAR(56)
                    REFERENCES soroban_contracts(contract_id) ON DELETE RESTRICT,
  caller_account    VARCHAR(69)
                    REFERENCES accounts(account_id) ON DELETE RESTRICT,
  function_name     VARCHAR(100) NOT NULL,
  successful        BOOLEAN NOT NULL,
  invocation_index  SMALLINT NOT NULL DEFAULT 0,
  ledger_sequence   BIGINT NOT NULL,
  created_at        TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (id, created_at),
  UNIQUE (transaction_id, invocation_index, created_at)
) PARTITION BY RANGE (created_at);
-- Dedup: ON CONFLICT (transaction_id, invocation_index, created_at) DO NOTHING.
```

#### 10. `tokens`

Token identity. Supply/holder_count in `token_supply_snapshots`. Metadata on S3.

```sql
tokens (
  id               BIGSERIAL PRIMARY KEY,
  asset_type       VARCHAR(20) NOT NULL,
  asset_code       VARCHAR(12),
  issuer_address   VARCHAR(56)
                   REFERENCES accounts(account_id) ON DELETE RESTRICT,
  contract_id      VARCHAR(56)
                   REFERENCES soroban_contracts(contract_id) ON DELETE RESTRICT,
  name             VARCHAR(256),
  metadata_ledger  BIGINT
)
-- Insert-once. Dedup via partial unique indexes:
-- idx_tokens_classic: UNIQUE (asset_code, issuer_address) WHERE asset_type IN ('classic','sac')
-- idx_tokens_soroban: UNIQUE (contract_id) WHERE asset_type = 'soroban'
-- idx_tokens_sac:     UNIQUE (contract_id) WHERE asset_type = 'sac'
-- NOTE: promoted from SERIAL to BIGSERIAL — safety margin for long-term growth
-- (INT4 exhaustion unrealistic, but BIGSERIAL costs 4 B more per row and removes the
-- only numeric-identity 32-bit ceiling in the schema).
```

#### 11. `token_supply_snapshots`

Insert-only history of token supply and holder count.

```sql
token_supply_snapshots (
  id               BIGSERIAL PRIMARY KEY,
  token_id         BIGINT NOT NULL
                   REFERENCES tokens(id) ON DELETE RESTRICT,
  ledger_sequence  BIGINT NOT NULL,
  total_supply     NUMERIC(39,0) NOT NULL,
  holder_count     INTEGER NOT NULL,
  UNIQUE (token_id, ledger_sequence)
)
-- Emitted when total_supply or holder_count changes for the token in a ledger.
-- Driven by the same balance-change events that populate account_balances.
-- Dedup: ON CONFLICT (token_id, ledger_sequence) DO NOTHING.
-- Reconstruction @ ledger X: ORDER BY ledger_sequence DESC LIMIT 1.
```

#### 12. `nfts`

NFT identity + lightweight display fields. Current owner is materialized in
`nft_current_ownership`. Full metadata on S3.

```sql
nfts (
  id                BIGSERIAL PRIMARY KEY,
  contract_id       VARCHAR(56) NOT NULL
                    REFERENCES soroban_contracts(contract_id) ON DELETE RESTRICT,
  token_id          VARCHAR(256) NOT NULL,
  collection_name   VARCHAR(256),
  name              VARCHAR(256),
  media_url         TEXT,
  minted_at_ledger  BIGINT,
  search_vector     TSVECTOR GENERATED ALWAYS AS
                    (to_tsvector('simple',
                       coalesce(name, '') || ' ' || coalesce(collection_name, ''))) STORED,
  UNIQUE (contract_id, token_id)
)
-- Insert-once. Name, media_url, minted_at_ledger filled via COALESCE.
-- Only fields required for list view (name, collection_name, media_url) kept in DB.
-- Full attribute metadata JSONB on S3: parsed_ledger_{minted_at_ledger}.json →
-- nft_metadata[{contract_id, token_id}].metadata
-- NOTE: promoted from SERIAL to BIGSERIAL (same rationale as tokens.id).
```

**Offloaded to S3:** `metadata` (full attribute payload). Consistent with `tokens.metadata`
and `soroban_contracts.metadata`. `GET /nfts/:id` detail view fetches the ledger file
via bridge column `minted_at_ledger` (1 S3 fetch per detail request).

#### 13. `nft_ownership`

Insert-only history of NFT ownership changes (mint, transfer, burn).

```sql
nft_ownership (
  id              BIGSERIAL PRIMARY KEY,
  nft_id          BIGINT NOT NULL
                  REFERENCES nfts(id) ON DELETE RESTRICT,
  transaction_id  BIGINT NOT NULL
                  REFERENCES transactions(id) ON DELETE RESTRICT,
  owner_account   VARCHAR(69)
                  REFERENCES accounts(account_id) ON DELETE RESTRICT,
  event_type      VARCHAR(20) NOT NULL,
  ledger_sequence BIGINT NOT NULL,
  event_order     SMALLINT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL,
  UNIQUE (nft_id, ledger_sequence, event_order)
)
-- owner_account NULL on burn.
-- event_order disambiguates multiple ownership changes in the same ledger.
-- Dedup: ON CONFLICT (nft_id, ledger_sequence, event_order) DO NOTHING.
```

#### 14. `liquidity_pools`

Pool identity + static config. Mutable state in `liquidity_pool_snapshots`.

```sql
liquidity_pools (
  pool_id           VARCHAR(64) PRIMARY KEY,
  asset_a_type      VARCHAR(20) NOT NULL,
  asset_a_code      VARCHAR(12),
  asset_a_issuer    VARCHAR(56)
                    REFERENCES accounts(account_id) ON DELETE RESTRICT,
  asset_b_type      VARCHAR(20) NOT NULL,
  asset_b_code      VARCHAR(12),
  asset_b_issuer    VARCHAR(56)
                    REFERENCES accounts(account_id) ON DELETE RESTRICT,
  fee_bps           INTEGER NOT NULL,
  created_at_ledger BIGINT NOT NULL
)
-- Immutable, insert-once.
```

#### 15. `liquidity_pool_snapshots`

Insert-only pool state history. Partitioned by `created_at`.

```sql
liquidity_pool_snapshots (
  id               BIGSERIAL,
  pool_id          VARCHAR(64) NOT NULL
                   REFERENCES liquidity_pools(pool_id) ON DELETE RESTRICT,
  ledger_sequence  BIGINT NOT NULL,
  created_at       TIMESTAMPTZ NOT NULL,
  reserve_a        NUMERIC(39,0) NOT NULL,
  reserve_b        NUMERIC(39,0) NOT NULL,
  total_shares     NUMERIC NOT NULL,
  tvl              NUMERIC,
  volume           NUMERIC,
  fee_revenue      NUMERIC,
  PRIMARY KEY (id, created_at),
  UNIQUE (pool_id, ledger_sequence, created_at)
) PARTITION BY RANGE (created_at);
-- Dedup: ON CONFLICT (pool_id, ledger_sequence, created_at) DO NOTHING.
```

#### 16. `wasm_interface_metadata`

Staging table bridging 2-ledger Soroban deploy pattern. Full metadata on S3.

```sql
wasm_interface_metadata (
  wasm_hash          VARCHAR(64) PRIMARY KEY,
  name               VARCHAR(256),
  uploaded_at_ledger BIGINT NOT NULL,
  contract_type      VARCHAR(50) NOT NULL DEFAULT 'other'
)
-- Insert-once per wasm_hash. Full WASM spec on S3:
-- parsed_ledger_{uploaded_at_ledger}.json → wasm_uploads[wasm_hash].
-- contract_type classified by parser ('nft', 'fungible', 'other').
-- Used by deploy step to populate soroban_contracts.name and contract_type.
```

### Activity projection tables

The event-log tables above answer chain-native questions ("what happened") but not
entity-centric API questions ("show me everything involving X"). Five endpoints in
`backend-overview.md` cannot be served efficiently from the event log alone:

- `GET /accounts/:id/transactions` — account is referenced from `transactions.source_account`,
  `operations.source_account`, `operations.destination`, `soroban_invocations.caller_account`,
  `nft_ownership.owner_account`, plus trustline/claimable parties. UNION across 6+ partitioned
  tables under cursor pagination is unusable.
- `GET /tokens/:id/transactions` — classic assets are identified by `(asset_code, asset_issuer)`
  pairs in `operations`; Soroban tokens live in `soroban_events`. No single query plan unifies.
- `GET /nfts/:id` current owner + "NFTs owned by X" — requires latest row per `nft_id` then
  filter by `owner_account`. `DISTINCT ON (nft_id)` over billions of ownership rows with a
  post-DISTINCT WHERE clause is not indexable.
- `GET /tokens` sort/filter by `holder_count`, `GET /liquidity-pools?filter[min_tvl]` — need
  latest snapshot per entity globally orderable.
- `GET /contracts/:id` stats (total invocations, unique callers) — `COUNT(DISTINCT)` over
  partitioned tables with billions of rows.

Projection tables are written during the persist phase alongside the event log, reading
from the structs the parser already produces. They are **derived** — rebuildable from the
event log and S3 via a reindex script — and are the only exception to the zero-upsert rule
(the `_current` projections upsert by design; the activity feeds are append-only).

#### 17. `account_activity`

Denormalized feed of every (account, transaction) touchpoint. One row per role played.

```sql
account_activity (
  account_id      VARCHAR(69) NOT NULL
                  REFERENCES accounts(account_id) ON DELETE RESTRICT,
  transaction_id  BIGINT NOT NULL
                  REFERENCES transactions(id) ON DELETE RESTRICT,
  ledger_sequence BIGINT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL,
  role            VARCHAR(20) NOT NULL,
  PRIMARY KEY (account_id, transaction_id, role, created_at)
) PARTITION BY RANGE (created_at);
-- role ∈ {source, destination, caller, nft_owner, pool_party, trustor, claimant}.
-- An account may appear multiple times per tx under distinct roles; PK reflects that.
-- Append-only: dedup via ON CONFLICT DO NOTHING on the composite PK.
-- Emitted at persist time by collecting account references from all Extracted* structs
-- for the transaction.
```

#### 18. `token_activity`

Denormalized feed of every (token, transaction) touchpoint.

```sql
token_activity (
  token_id        BIGINT NOT NULL
                  REFERENCES tokens(id) ON DELETE RESTRICT,
  transaction_id  BIGINT NOT NULL
                  REFERENCES transactions(id) ON DELETE RESTRICT,
  ledger_sequence BIGINT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (token_id, transaction_id, created_at)
) PARTITION BY RANGE (created_at);
-- Unifies classic asset ops (operations.asset_code/asset_issuer → tokens.id lookup) and
-- Soroban token events (soroban_events.contract_id → tokens.id lookup).
-- Persist-time emission: for each tx, collect distinct tokens referenced across
-- operations + soroban_events, emit one row per (token, tx).
```

#### 19. `nft_current_ownership`

Upserted projection of the latest `nft_ownership` row per `nft_id`.

```sql
nft_current_ownership (
  nft_id          BIGINT PRIMARY KEY
                  REFERENCES nfts(id) ON DELETE RESTRICT,
  owner_account   VARCHAR(69)
                  REFERENCES accounts(account_id) ON DELETE RESTRICT,
  ledger_sequence BIGINT NOT NULL,
  transaction_id  BIGINT NOT NULL
                  REFERENCES transactions(id) ON DELETE RESTRICT
)
-- Watermark upsert: only replace when incoming ledger_sequence > stored ledger_sequence.
-- owner_account NULL on burn (matches nft_ownership semantics).
-- Rebuildable: TRUNCATE + INSERT SELECT DISTINCT ON (nft_id) from nft_ownership
-- ORDER BY nft_id, ledger_sequence DESC, event_order DESC.
```

#### 20. `token_current_supply`

Upserted projection of the latest `token_supply_snapshots` row per `token_id`.

```sql
token_current_supply (
  token_id        BIGINT PRIMARY KEY
                  REFERENCES tokens(id) ON DELETE RESTRICT,
  total_supply    NUMERIC(39,0) NOT NULL,
  holder_count    INTEGER NOT NULL,
  ledger_sequence BIGINT NOT NULL
)
-- Watermark upsert. Rebuildable from token_supply_snapshots.
-- Enables GET /tokens sort/filter by holder_count and total_supply without
-- latest-per-group reconstruction at read time.
```

#### 21. `liquidity_pool_current`

Upserted projection of the latest `liquidity_pool_snapshots` row per `pool_id`.

```sql
liquidity_pool_current (
  pool_id         VARCHAR(64) PRIMARY KEY
                  REFERENCES liquidity_pools(pool_id) ON DELETE RESTRICT,
  reserve_a       NUMERIC(39,0) NOT NULL,
  reserve_b       NUMERIC(39,0) NOT NULL,
  total_shares    NUMERIC NOT NULL,
  tvl             NUMERIC,
  volume_24h      NUMERIC,
  fee_revenue     NUMERIC,
  ledger_sequence BIGINT NOT NULL
)
-- Watermark upsert. Rebuildable from liquidity_pool_snapshots.
-- Enables GET /liquidity-pools filter[min_tvl] and sort-by-TVL.
-- volume_24h populated by a separate rollup job (not a parser-time emission).
```

#### 22. `contract_stats_daily`

Rollup of contract activity with HyperLogLog for cheap `COUNT(DISTINCT caller)`.

```sql
CREATE EXTENSION IF NOT EXISTS hll;

contract_stats_daily (
  contract_id      VARCHAR(56) NOT NULL
                   REFERENCES soroban_contracts(contract_id) ON DELETE RESTRICT,
  day              DATE NOT NULL,
  invocation_count BIGINT NOT NULL,
  unique_callers   hll NOT NULL,
  last_active_at   TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (contract_id, day)
)
-- Refreshed by scheduled job reading soroban_invocations partitions.
-- GET /contracts/:id reads SUM(invocation_count) + HLL merge of unique_callers.
-- Rebuildable from soroban_invocations.
```

#### 23. `search_index`

Unified search table replacing per-entity UNION ALL queries.

```sql
CREATE EXTENSION IF NOT EXISTS pg_trgm;

search_index (
  entity_type   VARCHAR(20) NOT NULL,
  entity_ref    VARCHAR(128) NOT NULL,
  search_key    VARCHAR(256) NOT NULL,
  display_label VARCHAR(256),
  search_tsv    TSVECTOR,
  rank_weight   SMALLINT NOT NULL DEFAULT 0,
  PRIMARY KEY (entity_type, entity_ref)
)
-- entity_type ∈ {tx, account, contract, token, nft, pool, ledger}.
-- entity_ref: primary key / composite reference for deep-link.
-- search_key: canonical lookup string (hash, address, asset_code, token_id, pool_id).
-- display_label: UI-friendly name (token name, NFT name, collection_name).
-- search_tsv: TSVECTOR for display_label full-text search.
-- rank_weight: higher = preferred result (exact hash > address prefix > fuzzy name).
-- Upserted at persist time alongside identity-table inserts.
```

### Foreign key graph summary

Parent → Children:

```
accounts ◄──── transactions.source_account
          ◄──── operations.source_account / destination
          ◄──── soroban_invocations.caller_account
          ◄──── nft_ownership.owner_account
          ◄──── nft_current_ownership.owner_account
          ◄──── account_balances.account_id
          ◄──── account_home_domain_changes.account_id
          ◄──── account_activity.account_id
          ◄──── soroban_contracts.deployer_account
          ◄──── tokens.issuer_address
          ◄──── liquidity_pools.asset_a_issuer / asset_b_issuer

soroban_contracts ◄──── nfts.contract_id
                   ◄──── tokens.contract_id
                   ◄──── operations.contract_id
                   ◄──── soroban_events.contract_id
                   ◄──── soroban_invocations.contract_id
                   ◄──── contract_stats_daily.contract_id

liquidity_pools ◄──── operations.pool_id
                 ◄──── liquidity_pool_snapshots.pool_id
                 ◄──── liquidity_pool_current.pool_id

tokens ◄──── token_supply_snapshots.token_id
        ◄──── token_current_supply.token_id
        ◄──── token_activity.token_id

nfts ◄──── nft_ownership.nft_id
      ◄──── nft_current_ownership.nft_id

wasm_interface_metadata ◄──── soroban_contracts.wasm_hash

transactions ◄──── operations.transaction_id
              ◄──── soroban_events.transaction_id
              ◄──── soroban_invocations.transaction_id
              ◄──── nft_ownership.transaction_id
              ◄──── nft_current_ownership.transaction_id
              ◄──── account_activity.transaction_id
              ◄──── token_activity.transaction_id
```

All FKs: `ON DELETE RESTRICT`. No CASCADE — on-chain data is immutable.

**Note: `ledgers` has no incoming FKs.** Columns like `ledger_sequence`,
`first_seen_ledger`, `deployed_at_ledger`, `minted_at_ledger`, etc. exist as plain
`BIGINT` columns without FK constraint. `ledgers` is treated as a **dimension table**,
not an entity hub. See "Why ledger_sequence is not a FK" for rationale.

Total FKs: **~34** (25 from the core graph + 9 from activity projections).

### Reconstruction queries

Every "current state" or "state @ ledger X" query has the same shape. For current-state
queries at the latest ledger, prefer the `_current` projections; use the full history
table only when explicitly querying past state.

```sql
-- Account state @ ledger X
SELECT first_seen_ledger FROM accounts WHERE account_id = ?;

SELECT source_post_sequence_number FROM transactions
WHERE source_account = ? AND ledger_sequence <= X AND successful
ORDER BY ledger_sequence DESC LIMIT 1;

SELECT home_domain FROM account_home_domain_changes
WHERE account_id = ? AND ledger_sequence <= X
ORDER BY ledger_sequence DESC LIMIT 1;

SELECT DISTINCT ON (asset_type, asset_code, issuer) balance
FROM account_balances
WHERE account_id = ? AND ledger_sequence <= X
ORDER BY asset_type, asset_code, issuer, ledger_sequence DESC, event_order DESC;

-- NFT owner @ ledger X (current: SELECT FROM nft_current_ownership)
SELECT owner_account FROM nft_ownership
WHERE nft_id = ? AND ledger_sequence <= X
ORDER BY ledger_sequence DESC, event_order DESC LIMIT 1;

-- Token supply @ ledger X (current: SELECT FROM token_current_supply)
SELECT total_supply, holder_count FROM token_supply_snapshots
WHERE token_id = ? AND ledger_sequence <= X
ORDER BY ledger_sequence DESC LIMIT 1;

-- Pool state @ ledger X (current: SELECT FROM liquidity_pool_current)
SELECT * FROM liquidity_pool_snapshots
WHERE pool_id = ? AND ledger_sequence <= X
ORDER BY ledger_sequence DESC LIMIT 1;
```

---

## Indexing strategy

### Build order: backfill-time vs post-backfill

Indexes are split into two sets by _when_ they are created:

- **Backfill-time** — only the indexes implicit in PK / UNIQUE / partial-UNIQUE declarations.
  These are required for `ON CONFLICT DO NOTHING` dedup and for FK validation of inbound
  writes. **No other indexes exist during backfill.**
- **Post-backfill** — every secondary index is built with `CREATE INDEX CONCURRENTLY`
  after the backfill watermark reaches network tip (or per sealed partition using the
  build-then-`ATTACH PARTITION` pattern during backfill to avoid the CONCURRENTLY cost on
  quiet partitions).

Rationale: full-index backfill costs ~6 index entries per row on the hottest tables
(`operations`, `soroban_events`). Dropping to ~2 entries (PK + UNIQUE only) recovers a
measured 2.5–3× in backfill wall-clock. This is ADR 0011's hot-path concern, preserved.

### Backfill-time minimum (implicit only)

Every table gets exactly the indexes auto-created by its PK and UNIQUE constraints:

```sql
-- Core tables: PK + UNIQUE as declared in the "Core tables — schema" section.
-- Partial unique indexes on tokens (already declared as part of the DDL):
CREATE UNIQUE INDEX idx_tokens_classic
  ON tokens (asset_code, issuer_address) WHERE asset_type IN ('classic','sac');
CREATE UNIQUE INDEX idx_tokens_soroban
  ON tokens (contract_id) WHERE asset_type = 'soroban';
CREATE UNIQUE INDEX idx_tokens_sac
  ON tokens (contract_id) WHERE asset_type = 'sac';

-- Extensions needed during backfill (GIN/trigram indexes build later):
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE EXTENSION IF NOT EXISTS hll;
```

**Table-level tuning for progressive-COALESCE tables** (required during backfill to keep
UPDATE paths in HOT mode):

```sql
ALTER TABLE soroban_contracts
  SET (fillfactor = 90, autovacuum_vacuum_scale_factor = 0.05);
ALTER TABLE nfts
  SET (fillfactor = 90, autovacuum_vacuum_scale_factor = 0.05);
ALTER TABLE tokens
  SET (fillfactor = 90, autovacuum_vacuum_scale_factor = 0.05);
-- All append-only tables retain default fillfactor = 100 (no UPDATEs ever).
```

### Post-backfill index set (CREATE INDEX CONCURRENTLY)

Built against the sealed data set once backfill reaches tip. For partitioned tables,
indexes are declared on the partitioned parent and auto-propagated to all partitions.
Hot-partition indexes run last via CONCURRENTLY; older partitions use build-then-attach.

#### `transactions`

```sql
CREATE INDEX CONCURRENTLY idx_tx_ledger_id_desc
  ON transactions (ledger_sequence DESC, id DESC);
-- GET /transactions list (cursor pagination newest-first).

CREATE INDEX CONCURRENTLY idx_tx_source_ledger_desc
  ON transactions (source_account, ledger_sequence DESC, id DESC);
-- GET /accounts/:id/transactions (source-role only; full feed via account_activity).

CREATE INDEX CONCURRENTLY idx_tx_source_post_seq
  ON transactions (source_account, ledger_sequence DESC)
  WHERE successful = TRUE;
-- Reconstruction: source_post_sequence_number @ ledger X.
-- Partial index — reconstruction query always adds AND successful.

CREATE INDEX CONCURRENTLY brin_tx_created_at
  ON transactions USING BRIN (created_at) WITH (pages_per_range = 128);
-- Cheap range-scan fallback for analytics and wide time windows.
```

#### `operations` (partitioned by `created_at`)

```sql
CREATE INDEX CONCURRENTLY idx_ops_contract_created
  ON operations (contract_id, created_at DESC) WHERE contract_id IS NOT NULL;
-- filter[contract_id] on /transactions; contract activity via operations.

CREATE INDEX CONCURRENTLY idx_ops_pool_created
  ON operations (pool_id, created_at DESC) WHERE pool_id IS NOT NULL;
-- GET /liquidity-pools/:id/transactions.

CREATE INDEX CONCURRENTLY idx_ops_source_created
  ON operations (source_account, created_at DESC);
-- Account-source feed component.

CREATE INDEX CONCURRENTLY idx_ops_destination_created
  ON operations (destination, created_at DESC) WHERE destination IS NOT NULL;
-- Account-destination feed component.

CREATE INDEX CONCURRENTLY idx_ops_asset_classic
  ON operations (asset_code, asset_issuer, created_at DESC) WHERE asset_code IS NOT NULL;
-- Classic-asset token activity (token_activity also serves this for unified view).

CREATE INDEX CONCURRENTLY idx_ops_type_created
  ON operations (type, created_at DESC);
-- filter[operation_type] on /transactions.
```

#### Account history

```sql
CREATE INDEX CONCURRENTLY idx_balances_recon
  ON account_balances
  (account_id, asset_type, asset_code, issuer, ledger_sequence DESC, event_order DESC);
-- Matches exactly the DISTINCT ON reconstruction query.
-- UNIQUE (account_id, ledger_sequence, asset_type, asset_code, issuer, event_order) is
-- kept as dedup guard only — wrong column order for the reconstruction query.

CREATE INDEX CONCURRENTLY brin_balances_ledger
  ON account_balances USING BRIN (ledger_sequence) WITH (pages_per_range = 64);
-- Range scans for analytics.

-- account_home_domain_changes: UNIQUE (account_id, ledger_sequence) + backward scan
-- is sufficient (single-column tail sort). No explicit DESC index needed.
```

#### Soroban

```sql
CREATE INDEX CONCURRENTLY idx_events_contract_created
  ON soroban_events (contract_id, created_at DESC);
-- GET /contracts/:id/events.

CREATE INDEX CONCURRENTLY idx_events_contract_topic0
  ON soroban_events (contract_id, topic0, created_at DESC) WHERE topic0 IS NOT NULL;
-- Event-name filtering within a contract (transfer/mint/burn drill-downs).

CREATE INDEX CONCURRENTLY idx_inv_contract_created
  ON soroban_invocations (contract_id, created_at DESC);
-- GET /contracts/:id/invocations.

CREATE INDEX CONCURRENTLY idx_inv_contract_fn
  ON soroban_invocations (contract_id, function_name, created_at DESC);
-- Per-function drill-down (e.g. all `swap` calls on a pool contract).

CREATE INDEX CONCURRENTLY idx_inv_caller_created
  ON soroban_invocations (caller_account, created_at DESC) WHERE caller_account IS NOT NULL;
-- Account-caller feed component.
```

#### Contracts

```sql
CREATE INDEX CONCURRENTLY idx_contracts_wasm_hash
  ON soroban_contracts (wasm_hash) WHERE wasm_hash IS NOT NULL;
CREATE INDEX CONCURRENTLY idx_contracts_deployer
  ON soroban_contracts (deployer_account) WHERE deployer_account IS NOT NULL;
CREATE INDEX CONCURRENTLY idx_contracts_type
  ON soroban_contracts (contract_type) WHERE contract_type IN ('nft','fungible','token');
CREATE INDEX CONCURRENTLY idx_contracts_search
  ON soroban_contracts USING GIN (search_vector);
```

#### NFTs

```sql
CREATE INDEX CONCURRENTLY idx_nfts_contract_id_desc
  ON nfts (contract_id, id DESC);
CREATE INDEX CONCURRENTLY idx_nfts_collection
  ON nfts (collection_name, id DESC) WHERE collection_name IS NOT NULL;
CREATE INDEX CONCURRENTLY idx_nfts_search
  ON nfts USING GIN (search_vector);

CREATE INDEX CONCURRENTLY idx_ownership_recon
  ON nft_ownership (nft_id, ledger_sequence DESC, event_order DESC);
-- Reconstruction: latest owner per NFT (use nft_current_ownership for current).
-- UNIQUE (nft_id, ledger_sequence, event_order) is kept only as dedup guard.

CREATE INDEX CONCURRENTLY idx_nft_current_owner
  ON nft_current_ownership (owner_account);
-- "NFTs currently owned by X" — account detail sidebar.
```

#### Tokens

```sql
CREATE INDEX CONCURRENTLY idx_tokens_type_code
  ON tokens (asset_type, asset_code);
CREATE INDEX CONCURRENTLY idx_tokens_issuer
  ON tokens (issuer_address) WHERE issuer_address IS NOT NULL;

CREATE INDEX CONCURRENTLY idx_supply_token_ledger
  ON token_supply_snapshots (token_id, ledger_sequence DESC);

CREATE INDEX CONCURRENTLY idx_token_current_holder
  ON token_current_supply (holder_count DESC);
CREATE INDEX CONCURRENTLY idx_token_current_supply
  ON token_current_supply (total_supply DESC);
-- Sort /tokens by holder_count or total_supply without reconstruction at read time.
```

#### Liquidity pools

```sql
CREATE INDEX CONCURRENTLY idx_pools_asset_a
  ON liquidity_pools (asset_a_code, asset_a_issuer) WHERE asset_a_code IS NOT NULL;
CREATE INDEX CONCURRENTLY idx_pools_asset_b
  ON liquidity_pools (asset_b_code, asset_b_issuer) WHERE asset_b_code IS NOT NULL;

CREATE INDEX CONCURRENTLY idx_pool_snap_pool_created
  ON liquidity_pool_snapshots (pool_id, created_at DESC);
-- GET /liquidity-pools/:id/chart and historical reconstruction.

CREATE INDEX CONCURRENTLY idx_pool_current_tvl
  ON liquidity_pool_current (tvl DESC) WHERE tvl IS NOT NULL;
-- filter[min_tvl] + sort-by-TVL on /liquidity-pools list.
```

#### Activity projections

```sql
CREATE INDEX CONCURRENTLY idx_acctact_account_time
  ON account_activity (account_id, created_at DESC, transaction_id DESC);
-- GET /accounts/:id/transactions (full feed across all roles).

CREATE INDEX CONCURRENTLY idx_acctact_account_role
  ON account_activity (account_id, role, created_at DESC);
-- Role-filtered variants (e.g. "only ops where account was caller").

CREATE INDEX CONCURRENTLY idx_tokact_token_time
  ON token_activity (token_id, created_at DESC, transaction_id DESC);
-- GET /tokens/:id/transactions (unified classic + Soroban feed).
```

#### Ledgers

```sql
CREATE INDEX CONCURRENTLY brin_ledgers_closed_at
  ON ledgers USING BRIN (closed_at) WITH (pages_per_range = 32);
-- GET /ledgers list default-sort. PK (sequence) already covers detail lookup.
-- BRIN over B-tree: ledgers is monotonically inserted — BRIN is ~1000× smaller.
```

#### `contract_stats_daily` / `search_index`

```sql
CREATE INDEX CONCURRENTLY idx_cstats_contract_day
  ON contract_stats_daily (contract_id, day DESC);

CREATE INDEX CONCURRENTLY idx_search_key_prefix
  ON search_index (search_key text_pattern_ops);
CREATE INDEX CONCURRENTLY idx_search_key_trgm
  ON search_index USING GIN (search_key gin_trgm_ops);
CREATE INDEX CONCURRENTLY idx_search_tsv
  ON search_index USING GIN (search_tsv);
```

### HOT preservation for progressive-COALESCE tables

PostgreSQL applies the HOT (Heap-Only Tuple) fast-path when:

1. The updated row fits on the same page (fillfactor headroom), AND
2. No index covers an updated column.

Three tables receive progressive COALESCE UPDATEs during backfill:
`soroban_contracts` (name, wasm_hash, deployer_account, deployed_at_ledger, contract_type),
`nfts` (name, media_url, collection_name, minted_at_ledger), `tokens` (name, metadata_ledger).

**Strategy:** none of the secondary indexes on these columns exist during backfill.
Every COALESCE UPDATE runs as a HOT update — same-page tuple replacement, no new index
entries, dead-tuple reclaim by autovacuum. Once backfill ends and `idx_contracts_search`,
`idx_nfts_search`, etc. are built CONCURRENTLY, further COALESCE UPDATEs on those
columns become non-HOT — but these are rare (only live deploys, a handful per minute),
so the cost is negligible.

Without this strategy, the generated `search_vector` GIN index would kill HOT on every
`name` fill UPDATE, causing multi-GB dead-tuple bloat competing with writers.

### Write-path amplification analysis

Per-row write cost during backfill (PK/UNIQUE only):

| Table                                                                      | Index entries / row | Notes                                    |
| -------------------------------------------------------------------------- | ------------------- | ---------------------------------------- |
| `ledgers`                                                                  | 2                   | PK + hash UNIQUE                         |
| `accounts`                                                                 | 1                   | PK                                       |
| `soroban_contracts` / `liquidity_pools` / `wasm_interface_metadata`        | 1                   | PK                                       |
| `tokens`                                                                   | 2                   | PK + one matching partial UNIQUE         |
| `nfts`                                                                     | 2                   | PK + UNIQUE(contract_id, token_id)       |
| `transactions`                                                             | 2                   | PK + hash UNIQUE                         |
| `operations`                                                               | 2                   | partition-local PK + UNIQUE              |
| `soroban_events` / `soroban_invocations`                                   | 2                   | partition-local PK + UNIQUE              |
| `account_balances`                                                         | 2                   | PK + 6-col UNIQUE                        |
| `account_home_domain_changes` / `token_supply_snapshots` / `nft_ownership` | 2                   | PK + UNIQUE                              |
| `liquidity_pool_snapshots`                                                 | 2                   | partition-local PK + UNIQUE              |
| Activity projections                                                       | 1                   | PK only (no secondaries during backfill) |

Steady-state (post-backfill) amplification:

| Table               | Backfill N | Steady-state N | Ratio |
| ------------------- | ---------: | -------------: | ----: |
| transactions        |          2 |              5 |  2.5× |
| operations          |          2 |              8 |  4.0× |
| soroban_events      |          2 |              4 |  2.0× |
| soroban_invocations |          2 |              5 |  2.5× |
| account_balances    |          2 |              3 |  1.5× |
| nft_ownership       |          2 |              3 |  1.5× |

The 4× factor on `operations` is the largest amplifier. Building its 6 secondary indexes
during backfill instead of after would add roughly 3× to backfill wall-clock on the
hottest table in the schema — weeks of compute.

### Per-endpoint verification

| Endpoint                                   | Indexes serving it                                                                                     |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------ |
| `GET /network/stats`                       | Cached; DB backstop via `MAX(ledgers.sequence)` + stats table                                          |
| `GET /transactions`                        | `idx_tx_ledger_id_desc`                                                                                |
| `GET /transactions/:hash`                  | `transactions.hash` UNIQUE                                                                             |
| `GET /transactions` filter[source_account] | `idx_tx_source_ledger_desc`                                                                            |
| `GET /transactions` filter[contract_id]    | `idx_ops_contract_created` → tx join                                                                   |
| `GET /transactions` filter[operation_type] | `idx_ops_type_created`                                                                                 |
| `GET /ledgers`                             | `brin_ledgers_closed_at` (+ PK DESC)                                                                   |
| `GET /ledgers/:sequence`                   | S3 `ledger_metadata` header (no DB hit)                                                                |
| `GET /accounts/:id`                        | `accounts` PK + `idx_balances_recon` + `account_home_domain_changes` UNIQUE + `idx_tx_source_post_seq` |
| `GET /accounts/:id/transactions`           | `idx_acctact_account_time` (preferred) or `idx_tx_source_ledger_desc` (source-only fallback)           |
| `GET /contracts/:id`                       | `soroban_contracts` PK + `contract_stats_daily` rollup                                                 |
| `GET /contracts/:id/interface`             | `idx_contracts_wasm_hash` → S3 fetch                                                                   |
| `GET /contracts/:id/invocations`           | `idx_inv_contract_created`                                                                             |
| `GET /contracts/:id/events`                | `idx_events_contract_created` (+ `_topic0` for filters)                                                |
| `GET /tokens`                              | `idx_tokens_type_code` + `idx_token_current_holder` / `idx_token_current_supply` for sort              |
| `GET /tokens/:id`                          | `tokens` PK + `token_current_supply` PK                                                                |
| `GET /tokens/:id/transactions`             | `idx_tokact_token_time`                                                                                |
| `GET /nfts`                                | `idx_nfts_contract_id_desc` / `idx_nfts_collection`                                                    |
| `GET /nfts/:id`                            | `nfts` UNIQUE(contract_id, token_id) + `nft_current_ownership` PK                                      |
| `GET /nfts/:id/transfers`                  | `idx_ownership_recon`                                                                                  |
| `GET /liquidity-pools`                     | `idx_pool_current_tvl` + `idx_pools_asset_a/b`                                                         |
| `GET /liquidity-pools/:id`                 | PK + `liquidity_pool_current` PK                                                                       |
| `GET /liquidity-pools/:id/transactions`    | `idx_ops_pool_created`                                                                                 |
| `GET /liquidity-pools/:id/chart`           | `idx_pool_snap_pool_created`                                                                           |
| `GET /search?q=`                           | `idx_search_key_prefix` / `idx_search_key_trgm` / `idx_search_tsv`                                     |

### Index footprint estimate at mainnet scale

| Component                                                      | ~Size       |
| -------------------------------------------------------------- | ----------- |
| Implicit PK / UNIQUE / partial UNIQUE (free)                   | ~60 GB      |
| B-tree secondaries (post-backfill)                             | ~150 GB     |
| BRIN on monotonic ledger/time columns                          | ~500 MB     |
| GIN (2× `search_vector` + `search_index` search_tsv + trigram) | ~15 GB      |
| Activity projections + their indexes                           | ~40 GB      |
| **Total**                                                      | **~265 GB** |

For comparison: naive "index every endpoint predicate with a full B-tree including
INCLUDE covering columns" projects to ~600 GB; a minimalist "only what SELECTs break
without" set projects to ~175 GB but leaves several endpoints structurally unserved.
265 GB is the chosen middle ground.

---

## Rationale

### Why zero upserts on mutable state

Block explorers historically answer "what is it now?" questions. Per-block
reconstructability ("what was it at ledger X?") is a strictly more powerful property:

- It enables future features (balance history, ownership history, supply history) without
  schema migration
- It makes the DB a true event log, eliminating a class of bugs where current state
  drifts from history
- It simplifies reasoning — every piece of mutable state has a single source of truth
  (the history table), no synchronization between denormalized "current" and "history"

The cost (extra storage, more JOINs in API queries) is modest:

- **Net ~17 GB LESS** on mainnet vs. ADR 0011 design. Zero-upsert history tables add
  ~820 MB, but offloading `transactions.memo`/`memo_type`/`result_code` and
  `nfts.metadata` to S3 saves ~18 GB. Overall core schema is lighter than ADR 0011.
- Current-state queries become `ORDER BY DESC LIMIT 1` — or a direct read from a
  `_current` projection table for endpoints that can't tolerate the sort cost at scale.

### Why activity projections (not just indexes on the event log)

Several endpoints (enumerated in Context point 3) fundamentally cannot be served by the
event log alone at mainnet scale. Even with perfect indexes on `transactions`,
`operations`, `soroban_events`, etc., the queries "all txs involving account X" and
"all txs touching token Y" require either:

- A **UNION across 5+ partitioned tables** with cross-partition ORDER BY and cursor
  pagination — multi-second p95 at mainnet scale
- A **materialized view** refreshed periodically — lag and refresh cost
- A **denormalized activity table** written at persist time — chosen option

The chosen option is the Etherscan pattern (`tx_by_address`, `token_tx`, `nft_tx`). Cost:
~40 GB of extra storage; write amplification of 2–3 extra index entries per transaction
(one `account_activity` row per distinct account touched, avg ~3; one `token_activity`
row per distinct token touched, avg ~1). Benefit: p95 for these endpoints drops from
unservable to &lt;10 ms.

The `_current` projections (`nft_current_ownership`, `token_current_supply`,
`liquidity_pool_current`) solve a different problem: "latest row per entity, globally
orderable". `DISTINCT ON` + `ORDER BY holder_count DESC` cannot be served by any index
on the source history table — the filter predicate depends on a derived aggregate. The
`_current` projection materializes the aggregate so a plain B-tree on `holder_count DESC`
works.

### Why deferred (post-backfill) index build

Backfill throughput is bounded by index maintenance cost. Adding all secondary indexes
from day one would triple per-row insert cost on the hottest table (`operations` goes
from 2 to 8 index entries per row). At 1B+ rows this is weeks of compute.

Deferring to `CREATE INDEX CONCURRENTLY` post-backfill (or build-then-`ATTACH PARTITION`
on sealed partitions) amortizes that cost over the build-once lifetime of the index
rather than on every row insert. The tradeoff: the API cannot go live until the
post-backfill build completes. Given that mainnet launch is gated on backfill-to-tip
anyway, this is an acceptable phasing.

### Why BRIN on ledger/time columns

`ledger_sequence` and `created_at` are strictly monotonically correlated with heap
insert order in live ingestion (single-writer, ordered ledger close). BRIN (Block Range
INdex) stores one summary per range of heap pages and is ~1000× smaller than the
equivalent B-tree. On a 2B-row `soroban_events` table, that's ~80 MB BRIN vs. ~65 GB
B-tree.

BRIN does not support: uniqueness, point lookups, ORDER BY ... LIMIT small. Keep B-tree
for those use cases; BRIN is additive, not a replacement. The planner chooses whichever
index is better per query.

Risk: parallel backfill workers writing disjoint ledger ranges interleave on the heap,
potentially degrading BRIN correlation. Mitigation: per-worker staging tables merged via
`INSERT ... SELECT ... ORDER BY ledger_sequence` into the final table, restoring
correlation to ~1.0. Acceptable fallback: live with correlation ~0.6 during backfill;
post-backfill live ingestion restores correlation to ~1.0 for all new data.

### Why partial indexes where used

Partial indexes with `WHERE column IS NOT NULL` cut index size 40–70% on nullable FK
columns (`contract_id`, `pool_id`, `destination`, `asset_code`, `caller_account`). Since
the endpoint query always includes `column = ?` (which implies `NOT NULL`), the planner
uses the partial index for every relevant query and skips the null-heavy rows entirely.

`WHERE successful = TRUE` on `idx_tx_source_post_seq` is justified because the ADR
reconstruction query always includes `AND successful`. It would be unsafe on generic
source-account filtering (which does include failed txs) — that path uses the full
`idx_tx_source_ledger_desc` instead.

`WHERE contract_type IN ('nft','fungible','token')` on `idx_contracts_type` excludes
the default `'other'` classification (post-backfill cleanup DELETE path in task 0118).
This is an enumerated partial rather than a boolean partial; the enum values are
explicit rather than conditional on a future value.

### Why no covering (INCLUDE) indexes by default

INCLUDE clauses enable index-only scans but roughly double index size and require the
visibility map to stay fresh via aggressive autovacuum. At 600 GB of total covering-index
bloat (projected for full IOS coverage), the operational cost outweighs the p95 win for
most endpoints.

Exceptions granted case-by-case after production EXPLAIN ANALYZE shows heap-fetch as the
bottleneck on a specific hot endpoint. The endpoint inventory does not yet indicate any
such hot-spot.

### Why full FK graph

Without FKs, schema expresses a wish ("this column holds a contract_id") but not a
contract ("this contract_id must exist"). At mainnet scale with a complex parser, bugs
that create orphan references become silent data corruption — detectable only by reader
queries returning empty results.

FKs make the DB actively protect the data model. The cost is:

- Slightly slower inserts (FK validation per row)
- Need for ordered ingestion (identity rows before facts)

Both are absorbed by the identity-first ingestion pattern with minimal throughput impact.

### Why `ON DELETE RESTRICT` everywhere

On-chain data is immutable. A successful DELETE on `transactions` would represent a
database corruption event, not a valid operation. RESTRICT ensures that even if a
misconfigured admin script attempts DELETE, it fails loudly rather than cascading.

### Why `ledger_sequence` is not a FK (dimensional modeling)

Every table except `ledgers` has a `ledger_sequence` (or `*_at_ledger`) column. The
earlier design made each of these a FK to `ledgers(sequence)`, producing 14 incoming
FKs on `ledgers` — 36% of the entire FK graph.

This ADR deliberately **does not** make these columns FKs. Rationale:

**1. `ledgers` is a dimension, not an entity.**
In data-warehouse terms, `ledgers` plays the role of `date_dim` — a lookup for "when
did this happen", not a parent entity whose existence must be checked at write time.
Facts and history rows reference a moment in time; they don't reference a ledger as a
business entity.

Contrast with `accounts` / `soroban_contracts` / `liquidity_pools`: those are true
entities. A row referring to `contract_id = CABC…` that doesn't exist is a real bug.
A row with `ledger_sequence = 12345` that doesn't yet have a `ledgers` row is just a
timing artifact — the indexer will create the ledger row eventually.

**2. Integrity value is close to zero.**
Parser produces `ledger_sequence` from `LedgerCloseMeta.header.ledger_seq` — an
authoritative protocol source. There is no string-matching fuzziness (like with account
addresses or contract IDs) that could produce a "near-miss" bad value. A FK would catch
only the pathological case of the parser emitting a nonsense BIGINT — which won't
happen.

**3. Parallel backfill simplifies significantly.**
With FK on every `ledger_sequence`, each worker had to ensure the `ledgers` row existed
before inserting any child data. That forces ordering and creates lock contention.
Without FK, workers can write their per-ledger payload independently; the `ledgers`
identity row can arrive at any time during or after.

**4. Insert throughput improves ~5-10%.**
14 FK checks removed from the hot path of every row insert in `operations`,
`transactions`, `soroban_events`, `account_balances`, etc. At billions of rows during
backfill this is measurable.

**5. Diagram clarity.**
14 FKs pointing at `ledgers` collapses to zero visible relationships. The hub-and-spoke
visual noise disappears. The diagram shows only real entity-to-entity relationships.

**Lost guarantee:** A `child.ledger_sequence` value could technically reference a ledger
that doesn't yet exist in the `ledgers` table. Mitigations:

- **Parser invariant** — `ledger_sequence` is always extracted from the ledger header
  the parser is currently processing, never guessed or reconstructed.
- **Monitoring healthcheck** — periodic query `SELECT MAX(ledger_sequence) FROM
account_balances WHERE ledger_sequence > (SELECT MAX(sequence) FROM ledgers)`
  detects any drift. Trivial to run as CloudWatch alarm.
- **No user-facing impact** — queries joining on `ledger_sequence` work by value
  matching regardless of FK presence.

**Retained FKs: business-entity relationships only.**
`account_id`, `contract_id`, `pool_id`, `wasm_hash`, `token_id`, `nft_id`,
`transaction_id` — these reference real entities where integrity catches real bugs.

This is a deliberate application of **Kimball dimensional modeling**: fact tables FK to
entity dimensions, but date/time dimensions are typically value-joined without enforced
constraints. `ledgers` is the time dimension of this schema.

### Why `operations` partitions by `created_at` (not `transaction_id`)

ADR 0011 partitioned `operations` by `transaction_id` for write distribution. That choice
makes every time-ranged or entity-filtered read query (`WHERE contract_id=?`,
`WHERE pool_id=?`, `WHERE source_account=?`) scan all partitions — no partition pruning
possible because `transaction_id` is a surrogate unrelated to time or entity. Aligning
with the partitioning of `soroban_events` and `soroban_invocations` (both by `created_at`)
restores partition pruning for time-filtered queries and unifies operational patterns
(same partition management job across all three fact tables).

### Why no CHECK constraints on VARCHAR

Stellar/Soroban protocol evolves: new operation types, new event categories, new
contract patterns. A CHECK whitelist requires a migration every time. The tradeoff:

- **Gain:** flexibility — parser can emit any string for `type`, `event_type`,
  `contract_type`; no migration for new protocol features
- **Cost:** consistency enforcement moves to parser/API layer — typos pass silently

Given Stellar's active protocol evolution and Soroban's early lifecycle, flexibility
wins. Consistency is a parser-layer concern.

### Why `search_index` instead of per-entity UNION ALL

`/search?q=` hits six entity types (transactions, contracts, tokens, accounts, NFTs,
pools) with both exact-match (hashes, IDs) and fuzzy (names). A naive UNION ALL across
six queries with individual LIMITs and outer ORDER BY measures at 80–300 ms p95 at
mainnet scale — each branch plans independently, GIN lossy recheck touches heap on each
branch.

A single materialized `search_index` table with one GIN-trigram index and one TSVECTOR
GIN brings this to 5–15 ms p95. Cost: ~15 GB storage + one extra INSERT per identity row
(rare — identities are insert-once). Benefit: autocomplete-latency search on a hot
endpoint.

### Why identity tables use surrogate `BIGSERIAL` for some, natural keys for others

Natural keys (`contract_id`, `pool_id`) used where:

- The value is already in every child row (contract addresses are on-chain identifiers
  referenced everywhere)
- No normalization benefit from a surrogate

Surrogate `BIGSERIAL` used where:

- FK from a large child table would otherwise duplicate a long string (e.g.
  `nft_ownership.nft_id BIGINT` vs. `(contract_id VARCHAR(56), token_id VARCHAR(256))`
  across 500K+ rows)
- Identity is conceptual, not inherent in on-chain data

Previously some identity tables used `SERIAL` (INT4, 2.1B ceiling). Promoted to
`BIGSERIAL` (INT8) across the board to eliminate the only 32-bit integer ceiling in the
schema. The extra 4 bytes per row is immaterial at mainnet row counts.

---

## Alternatives Considered

### Alternative 1: Keep ADR 0011 design with watermark upserts

**Description:** The previous ADR accepted upserts on `accounts`, `nfts.current_owner`,
and COALESCE-upserts on `soroban_contracts`.

**Pros:** simpler query model, fewer tables.

**Cons:** historical states for `sequence_number`, `home_domain`, NFT ownership trail are
unrecoverable; denormalized `current_owner` must be kept in sync with `nft_ownership` —
two sources of truth, classic race condition surface.

**Decision:** REJECTED — per-block reconstructability is a core requirement.

### Alternative 2: Full history for every mutable column including `sequence_number`

**Description:** Dedicated `account_states` insert-only table storing `sequence_number`
and `home_domain` snapshots per ledger per account.

**Pros:** symmetric treatment of all mutable account state.

**Cons:** `sequence_number` increments on every transaction from the account — generates
rows that duplicate information already in `transactions`. At mainnet scale: ~100M+ rows
added vs. ~800 MB from `transactions.source_post_sequence_number`.

**Decision:** REJECTED — derive from `transactions.source_post_sequence_number` achieves
the same reconstructability with 10× less storage and zero duplication.

### Alternative 3: No FKs (ADR 0011 approach)

**Description:** Omit FKs to avoid parallel-backfill ordering constraints.

**Pros:** simpler ingestion.

**Cons:** no DB-level integrity — parser bugs create silent data corruption.

**Decision:** REJECTED — identity-first pattern solves the ordering problem without
sacrificing integrity.

### Alternative 4: `DEFERRABLE INITIALLY DEFERRED` FKs

**Description:** FKs validated at transaction commit, not per-row.

**Pros:** workers can insert children before parents within a single transaction.

**Cons:** still requires parent to exist by commit — doesn't solve cross-worker ordering.

**Decision:** REJECTED — identity-first pattern is simpler and cross-worker-safe.

### Alternative 5: `NOT VALID` FKs during backfill, `VALIDATE` after

**Description:** Add FK without checking existing rows, validate post-backfill.

**Pros:** no ordering concerns during backfill.

**Cons:** FK enforcement absent during the longest-running phase; orphan rows created
during backfill are only detected at VALIDATE time, making recovery expensive.

**Decision:** REJECTED — identity-first pattern works uniformly for backfill and live.

### Alternative 6: No activity projections — rely on indexes over the event log

**Description:** Skip `account_activity`, `token_activity`, `_current` projections, and
instead rely on clever composite indexes and UNION queries in the API layer.

**Pros:** fewer tables, smaller schema.

**Cons:** multi-second p95 on `/accounts/:id/transactions`, `/tokens/:id/transactions`.
No index can resolve the "latest per entity globally orderable" query shape that
`_current` projections solve. Unifying classic-asset and Soroban-token activity in a
single feed requires a runtime UNION that negates partition pruning.

**Decision:** REJECTED — the structural missing piece is denormalization, not indexing.

### Alternative 7: Covering (INCLUDE) indexes on every list endpoint

**Description:** Add INCLUDE clauses to every list-endpoint index for index-only scans.

**Pros:** eliminates heap fetches on list endpoints; 5 ms → &lt;1 ms on select queries.

**Cons:** ~600 GB of index bloat (projected). Requires aggressive autovacuum tuning to
keep the visibility map fresh or IOS silently degrades. Operational complexity high.

**Decision:** REJECTED as a default; granted case-by-case after production EXPLAIN
ANALYZE shows a specific heap-fetch bottleneck.

### Alternative 8: UNION ALL across entity tables for `/search`

**Description:** Implement `/search` as a runtime UNION ALL across `transactions`,
`accounts`, `soroban_contracts`, `tokens`, `nfts`, `liquidity_pools`.

**Pros:** no extra table, no denormalization.

**Cons:** measured at 80–300 ms p95 at mainnet scale. Planner cannot push global LIMIT
down to individual branches; ranking across heterogeneous types is ad-hoc.

**Decision:** REJECTED in favor of the `search_index` materialized table.

### Alternative 9: All indexes built at backfill time

**Description:** Create every post-backfill index up front so the API can go live as
soon as backfill completes.

**Pros:** no index-build phase between backfill and API cutover.

**Cons:** 2.5–3× backfill wall-clock. At 2 years of mainnet data, this is weeks of
compute. HOT cannot apply on progressive-COALESCE tables, causing multi-GB bloat.

**Decision:** REJECTED — the deferred build is a clear net operational win.

---

## Consequences

### Positive

- **Per-block reconstructability** for every user-visible mutable state: account state,
  balances, NFT ownership, token supply, pool state
- **Full referential integrity** enforced at DB layer — parser bugs caught at insertion,
  not at read time
- **Zero denormalized current-state (in the event log)** — event-log tables have a
  single source of truth per concept; `_current` projections are explicit derived views
- **Simpler mental model** — every "current state" query has the same shape
  (`ORDER BY ledger_sequence DESC LIMIT 1`), or reads from a `_current` projection
- **Simpler write path** — event-log tables are inserts only; projections are
  watermark-upserts with clear "only replace if newer" semantics
- **All endpoints serviceable at p95 &lt; 200 ms** — activity projections + targeted
  indexing cover the API surface
- **Protocol flexibility** — no CHECK constraints means no migration when Stellar/Soroban
  adds new operation types or event categories
- **Consistent DB/S3 split** — no heavy JSONB in DB; all variable-structure metadata
  lives in S3 ledger files
- **`GET /ledgers/:sequence` served from a single S3 fetch** — skips DB entirely
- **Dimensional modeling for time** — `ledgers` is a dimension table; 14 FKs removed
- **Backfill throughput preserved** — deferred index build keeps amplification at 2×
  during backfill vs. 6× if all indexes existed from day one
- **HOT preserved on progressive-COALESCE tables** — no multi-GB dead-tuple bloat

### Negative

- **More tables** — 23 tables total (16 core + 7 projections) vs. a simpler upsert design
- **Projection-population cost at persist time** — each transaction emits ~4 extra rows
  on average (account_activity, token_activity, projection upserts). Measured at
  ~15–20% steady-state persist-path overhead. Acceptable for endpoint serviceability.
- **More JOINs in API queries** — account detail endpoint reads from 4 tables (identity,
  balances, home_domain, activity) vs. single-row SELECT in upsert design
- **Ingestion requires strict ordering** — workers must follow identity-first pattern
- **Storage** — net ~250 GB DB including indexes and projections at mainnet scale.
  Roughly 2× ADR 0011 estimate; justified by query serviceability
- **Parser-layer responsibility for value consistency** — enum-like columns not
  validated by DB
- **Write-path amplification** — every token balance change may produce a
  `token_supply_snapshots` row + a `token_current_supply` upsert. Mitigation via
  emission-only-on-change is a future optimization.
- **API cutover gated on post-backfill index build** — backfill-to-tip + CONCURRENTLY
  build must both complete before launch. Typical hot-partition CONCURRENTLY build is
  ~2–4 hours per index at mainnet scale; old partitions use faster ATTACH pattern.
- **Two query shapes per mutable-state endpoint** — `_current` projection for "now",
  history table for "@ ledger X". API layer routes based on presence of an `as_of`
  parameter.

### Per-block history coverage

**Full per-block history** for:

- Account balances, home_domain
- NFT ownership
- Token supply, holder count
- Liquidity pool state

**Immutable (no history needed)** — inherently single-state or append-only:

- `ledgers`, `transactions`, `operations`, `soroban_events`, `soroban_invocations`,
  `liquidity_pools`, `tokens`, `nfts`, `wasm_interface_metadata`

**Derivable from immutable data:**

- Account `sequence_number` (from `transactions.source_post_sequence_number`)
- Account `last_seen_ledger` (from `MAX(transactions.ledger_sequence)`)
- NFT `current_owner` (latest `nft_ownership` entry; projected into `nft_current_ownership`)
- Pool current state (latest `liquidity_pool_snapshots` entry; projected into `liquidity_pool_current`)
- Token current supply (latest `token_supply_snapshots` entry; projected into `token_current_supply`)

**Deferred/soft state** (COALESCE progressive fill, never overwrite):

- `soroban_contracts` stub → `name`, `wasm_hash`, `deployer_account`,
  `deployed_at_ledger`, `contract_type` (fill in as workers see deploy ledger)
- `nfts` stub → `name`, `media_url`, `collection_name`, `minted_at_ledger` (full metadata on S3)
- `tokens` → similar stub pattern for Soroban tokens

In all deferred cases, values are immutable on-chain — COALESCE fills NULLs, never
changes known values.

**Rebuildable projections** (derived from event log + S3, droppable and reconstructable
without data loss):

- `nft_current_ownership`, `token_current_supply`, `liquidity_pool_current`
- `account_activity`, `token_activity`
- `contract_stats_daily`
- `search_index`

---

## References

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md) — S3 offload principles inherited
- [ADR 0004: Rust-only XDR parsing](0004_rust-only-xdr-parsing.md) — parser produces pre-computed values for DB persist
- [ADR 0005: Rust-only backend API](0005_rust-only-backend-api.md) — API layer composes reconstruction queries
- [Database Audit](../../docs/database-audit-first-implementation.md) — table-by-table audit of current schema
- [Backend Overview](../../docs/architecture/backend/backend-overview.md) — endpoint inventory
- [SEP-0023: Muxed Account addresses](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0023.md)
- [SEP-0041: Soroban Token Interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md)
- [SEP-0050: Non-Fungible Tokens](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0050.md)
- [Etherscan Account Balance Checker](https://etherscan.io/balancecheck-tool) — historical balance feature reference
- [Etherscan tx_by_address pattern](https://etherscan.io/apis#accounts) — activity projection reference
