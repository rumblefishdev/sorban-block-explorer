---
id: '0011'
title: 'S3 offload: lightweight DB schema with parsed JSON on S3'
status: proposed
deciders: [fmazur]
related_tasks: []
related_adrs: ['0004', '0005', '0012']
tags: [database, s3, architecture, cost-optimization]
links: []
history:
  - date: 2026-04-16
    status: proposed
    who: fmazur
    note: 'ADR created — going through tables one by one'
---

# ADR 0011: S3 offload — lightweight DB schema with parsed JSON on S3

**Related:**

- [ADR 0004: Rust-only XDR parsing](0004_rust-only-xdr-parsing.md)
- [ADR 0005: Rust-only backend API](0005_rust-only-backend-api.md)

---

## Context

The current schema stores all parsed XDR data directly in PostgreSQL — full transaction
envelopes, result metadata, operation details, event payloads, invocation arguments, etc.

Measured on 100 indexed ledgers (343 MB total DB size):

| Table            | Total size | Heavy data % |
| ---------------- | ---------- | ------------ |
| `transactions`   | 139 MB     | 97.5%        |
| `soroban_events` | 136 MB     | 74.9%        |
| `operations`     | 44 MB      | 73.7%        |
| `accounts`       | 7.5 MB     | 64.6%        |

~87% of DB size is JSONB/TEXT blobs used only in detail views, never for filtering or
pagination. At mainnet scale (~75M ledgers) this projects to **2-5 TB** in RDS, of which
**1.7-4.3 TB** is heavy data that could live on S3 at a fraction of the cost.

The change: XDR parser writes parsed JSON files to S3. DB keeps only lightweight index
data (IDs, hashes, timestamps, filter columns). API endpoints first query DB for
routing/filtering, then fetch detail data from S3 when needed.

---

## Decision

### General approach

**Ingestion pipeline** has two distinct phases:

1. **Parse phase** — XDR parser processes a ledger and produces a complete parsed JSON
   in memory. All derived data (classification, extracted columns, bridge references) is
   computed here. The parser is the only place where full data is available — once this
   phase completes, the heavy data is never re-processed.

2. **Persist phase** — runs in parallel:
   - **S3 PUT** `parsed_ledger_{sequence}.json` — the full parsed JSON, as-is from the
     parser. This is write-once, immutable.
   - **DB INSERT/UPSERT** — lightweight columns extracted from the same parsed JSON.
     The DB persist step receives ready-to-write values from the parser; it does NOT
     re-analyze or transform data. All classification (e.g., `contract_type` for task 0118) and extraction (e.g., `name`, `topic0`, filter columns) happens in the parse
     phase.

**Consequence:** Any logic that requires full data (WASM function signatures, operation
details, event payloads) must run in the parse phase. The DB persist step only writes
pre-computed values.

- **DB** retains only columns needed for: filtering, pagination, sorting, JOINs, FKs,
  and list-view display. Every detail endpoint has a `ledger_sequence` column in DB
  that serves as bridge to the corresponding S3 file.
- **API detail endpoints** merge DB index data + S3 detail data.
- **API list endpoints** serve from DB only (no S3 round-trip).

**Column sizing convention — numeric amounts:**
Columns storing Stellar/Soroban token amounts use `NUMERIC(39,0)` — raw i128 integers
(SEP-0041 max: ~1.7×10^38 = 39 digits). Display formatting (decimal places) is done in
the API layer using each contract's `decimals()` value. Columns with undefined or
computed precision (`total_shares`, `tvl`, `volume`, `fee_revenue`) use bare `NUMERIC`.
PostgreSQL NUMERIC storage is variable-length regardless of declared precision — no
size difference.

**Column sizing convention — account addresses:**
All columns storing Stellar account addresses use `VARCHAR(69)` instead of `VARCHAR(56)`.
G-addresses (ed25519) are 56 chars, but muxed M-addresses (SEP-0023) are 69 chars.
M-addresses can appear as `source_account`, `destination`, `caller_account`, `owner_account`,
`deployer_account` in on-chain data. Contract addresses (C-prefixed) remain `VARCHAR(56)` —
contracts cannot be muxed.

### S3 file structure

One file per ledger: `parsed_ledger_{sequence}.json`

```json
{
  "ledger_sequence": 12345,
  "transactions": [
    {
      "hash": "abc123...",
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
      "wasm_hash": "def456...",
      "functions": [{"name": "swap", "inputs": [...], "outputs": [...]}],
      "wasm_byte_len": 45230,
      "name": "Soroswap Router"
    }
  ],
  "contract_metadata": [
    {
      "contract_id": "CABC...",
      "metadata": {...}
    }
  ],
  "token_metadata": [
    {
      "token_id": 42,
      "metadata": {...}
    }
  ]
}
```

**Write path:** Parser processes one ledger → dumps everything to one file → S3 PUT.
**Read path:** Every detail endpoint uses `ledger_sequence` from DB as bridge to locate
the correct S3 file.

**File size:** At Stellar's current protocol limits (~1000 ops/ledger, ~400-500 tx on busy
ledgers), a single parsed JSON file is estimated at 5-20 MB worst case. Detail endpoints
download the full file and extract one transaction by hash — acceptable latency given Lambda
memory (512 MB+) and API Gateway response caching on popular ledgers. If file sizes grow
significantly (e.g., after 5000 TPS upgrade), per-transaction files or byte-range indexing
can be added as a future optimization without schema changes.

### Table-by-table changes

#### 1. `ledgers` — NO CHANGES

Already fully lightweight (6 columns, ~96 B/row, 16 KB on 100 ledgers).
All columns needed by `GET /ledgers`, `GET /ledgers/:sequence`, `GET /network/stats`.

#### 2. `transactions` — OFFLOAD 4 columns to S3

**Removed from DB:**

| Column                   | Avg/row | Reason                                                  |
| ------------------------ | ------- | ------------------------------------------------------- |
| `envelope_xdr` (TEXT)    | 794 B   | Only advanced detail view                               |
| `result_xdr` (TEXT)      | 193 B   | Only advanced detail view                               |
| `result_meta_xdr` (TEXT) | 4.9 KB  | Never returned to frontend; kept on S3 as archive/debug |
| `operation_tree` (JSONB) | 392 B   | Only detail view                                        |

**Retained in DB (11 columns):**

```sql
transactions (
  id              BIGSERIAL PRIMARY KEY,
  hash            VARCHAR(64) NOT NULL UNIQUE,
  ledger_sequence BIGINT NOT NULL REFERENCES ledgers(sequence),
  source_account  VARCHAR(69) NOT NULL,    -- 69: muxed M-addresses (SEP-0023)
  fee_charged     BIGINT NOT NULL,
  successful      BOOLEAN NOT NULL,
  result_code     VARCHAR(50),
  memo_type       VARCHAR(20),
  memo            TEXT,
  created_at      TIMESTAMPTZ NOT NULL,
  parse_error     BOOLEAN
)
```

**Reduction:** ~6.5 KB → ~195 B per row. Table data: 66 MB → ~7 MB on 100 ledgers.

**API flow:**

- `GET /transactions` (list) → DB only
- `GET /transactions/:hash` (normal detail) → DB + S3 `parsed_ledger_{ledger_sequence}.json` (operation_tree)
- `GET /transactions/:hash` (advanced) → DB + S3 (XDRs + operation_tree)
- Bridge column: `ledger_sequence` → locates S3 file

**Note:** `filter[contract_id]` and `filter[operation_type]` on `GET /transactions` require
a JOIN to `operations` — these columns live in `operations` (`contract_id`, `type`), not
in `transactions`. The JOIN is on `operations.transaction_id = transactions.id`. This is
DB-only (no S3), but adds query complexity compared to direct column filters.

**Note on partition scan:** `operations` is partitioned by `transaction_id`, so filtering by
`contract_id` or `type` scans all partitions (no pruning possible). Per-partition B-tree
indexes on these columns keep individual scans fast. Acceptable at current scale; same
mitigation path as event/invocation detail endpoints if partition count grows significantly.

#### 3. `operations` — OFFLOAD `details`, EXTRACT filter columns

**Removed from DB:**

| Column            | Avg/row            | Reason                                  |
| ----------------- | ------------------ | --------------------------------------- |
| `details` (JSONB) | 255 B (max 7.6 KB) | Only tx detail view; full payload on S3 |

**Added to DB (extracted from `details` for filtering):**

| Column          | Type         | When NOT NULL                                                                      | Overhead |
| --------------- | ------------ | ---------------------------------------------------------------------------------- | -------- |
| `destination`   | VARCHAR(69)  | PAYMENT, CREATE*ACCOUNT, PATH_PAYMENT*\*, ACCOUNT_MERGE (~38% rows)                | ~1.6 MB  |
| `contract_id`   | VARCHAR(56)  | INVOKE_HOST_FUNCTION (~17% rows)                                                   | ~710 KB  |
| `function_name` | VARCHAR(100) | INVOKE_HOST_FUNCTION (~17% rows)                                                   | ~190 KB  |
| `asset_code`    | VARCHAR(12)  | PAYMENT, CHANGE*TRUST, MANAGE*_*OFFER, PATH_PAYMENT*_ (~60% rows)                  | ~225 KB  |
| `asset_issuer`  | VARCHAR(69)  | Same as asset_code — needed to distinguish same-code tokens from different issuers | ~2.8 MB  |
| `pool_id`       | VARCHAR(64)  | LIQUIDITY_POOL_DEPOSIT, LIQUIDITY_POOL_WITHDRAW (~small % rows)                    | minimal  |

These are duplicates of fields in `details` — extracted solely for DB-level filtering.
GIN index on `details` is replaced by targeted B-tree indexes on these columns.

**New schema:**

```sql
operations (
  id                BIGSERIAL,
  transaction_id    BIGINT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
  application_order SMALLINT NOT NULL,
  source_account    VARCHAR(69) NOT NULL,    -- 69: muxed M-addresses (SEP-0023)
  type              VARCHAR(50) NOT NULL,
  destination       VARCHAR(69),             -- 69: muxed M-addresses (SEP-0023)
  contract_id       VARCHAR(56),
  function_name     VARCHAR(100),
  asset_code        VARCHAR(12),
  asset_issuer      VARCHAR(69),             -- needed to distinguish same-code tokens from different issuers
  pool_id           VARCHAR(64),
  created_at        TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (id, transaction_id),
  UNIQUE (transaction_id, application_order)
) PARTITION BY RANGE (transaction_id);
-- Dedup: ON CONFLICT (transaction_id, application_order) DO NOTHING
```

**S3:** `operations[].details` included in `parsed_ledger_{sequence}.json` → `transactions[].operations[].details`.

**Reduction:** net ~13 MB savings on 100 ledgers (19 MB removed, ~5.5 MB added incl. asset_issuer).

**API flow:**

- `GET /transactions` (list) → DB: COUNT, type filtering
- `GET /transactions/:hash` (detail) → DB lightweight cols + S3 full details
- `filter[operation_type]`, `filter[destination]`, etc. → DB indexed columns

#### 4. `soroban_contracts` — OFFLOAD `metadata` to S3, EXTRACT `name`

Small table (~913 rows, 336 KB on 100 ledgers) but applying the same principle:
DB = index for filtering/search, S3 = full data.

**Removed from DB:**

| Column             | Size          | Reason                                    |
| ------------------ | ------------- | ----------------------------------------- |
| `metadata` (JSONB) | 10-100 KB/row | Detail + interface endpoints; moved to S3 |

**Added to DB:**

| Column | Type         | Reason                                                 |
| ------ | ------------ | ------------------------------------------------------ |
| `name` | VARCHAR(256) | Extracted from metadata for `search_vector` generation |

`search_vector` remains as TSVECTOR GENERATED from `name` instead of `metadata->>'name'`.

**New schema:**

```sql
soroban_contracts (
  contract_id        VARCHAR(56) PRIMARY KEY,
  wasm_hash          VARCHAR(64),
  deployer_account   VARCHAR(69),             -- 69: muxed M-addresses (SEP-0023)
  deployed_at_ledger BIGINT REFERENCES ledgers(sequence),
  contract_type      VARCHAR(50),
  is_sac             BOOLEAN NOT NULL DEFAULT FALSE,
  name               VARCHAR(256),
  search_vector      TSVECTOR GENERATED ALWAYS AS (to_tsvector('simple', coalesce(name, ''))) STORED
)
```

**S3:** Contract metadata included in `parsed_ledger_{deployed_at_ledger}.json` →
`contract_metadata[]`. WASM function signatures in `parsed_ledger_{uploaded_at_ledger}.json`
→ `wasm_uploads[]`.

**Write path changes:**

- `upsert_contract_deployments_batch` → extracts `name` from metadata, writes to column
- `update_contract_interfaces_by_wasm_hash` → classifies `contract_type` from in-memory
  parsed WASM, then writes metadata to S3 as part of the ledger file
- `wasm_interface_metadata` staging table → metadata JSONB moves to S3 ledger file

**API flow:**

- `GET /contracts/:id` (detail) → DB (lightweight cols) + S3 `parsed_ledger_{deployed_at_ledger}.json`
- `GET /contracts/:id/interface` → DB: `wasm_hash` + `uploaded_at_ledger` from
  `wasm_interface_metadata` → S3 `parsed_ledger_{uploaded_at_ledger}.json` → `wasm_uploads[]`
- `GET /search?q=soroswap` → DB: search_vector (works — `name` in DB)
- Task 0118 NFT classification → DB: `contract_type` (unchanged)

#### 5. `soroban_events` — OFFLOAD `topics`+`data`, EXTRACT `topic0`

Densest table: 360K rows on 100 ledgers (3.6K/ledger). 60 MB of heavy JSONB data.

**Removed from DB:**

| Column           | Avg/row           | Reason                                           |
| ---------------- | ----------------- | ------------------------------------------------ |
| `topics` (JSONB) | 116 B (max 376 B) | Full payload on S3; GIN index replaced by topic0 |
| `data` (JSONB)   | 59 B (max 7.3 KB) | Only needed in detail views                      |

**Added to DB:**

| Column   | Type         | Reason                                                                 |
| -------- | ------------ | ---------------------------------------------------------------------- |
| `topic0` | VARCHAR(100) | First topic value (event name: "transfer", "mint", etc.) for filtering |

GIN index on `topics` is replaced by B-tree index on `topic0`.

**New schema:**

```sql
soroban_events (
  id               BIGSERIAL,
  transaction_id   BIGINT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
  contract_id      VARCHAR(56),
  event_type       VARCHAR(20) NOT NULL,
  topic0           VARCHAR(100),
  event_index      SMALLINT NOT NULL DEFAULT 0,
  ledger_sequence  BIGINT NOT NULL,   -- denormalized bridge to S3 (no FK to ledgers ��� parallel backfill safety)
  created_at       TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (id, created_at),
  UNIQUE (transaction_id, event_index, created_at)
) PARTITION BY RANGE (created_at);
-- Dedup: ON CONFLICT (transaction_id, event_index, created_at) DO NOTHING
-- Note: created_at in UNIQUE is required by PostgreSQL (partition key must be in all unique constraints)
-- Column-list form required: ON CONFLICT ON CONSTRAINT <name> does not work on partitioned tables
```

**S3:** `events[].topics` and `events[].data` included in `parsed_ledger_{sequence}.json`
→ `transactions[].events[]`.

**Reduction:** table from 136 MB → ~65 MB on 100 ledgers. Savings: ~71 MB.

**S3 fetch count per endpoint:**

- `GET /contracts/:id/events` (list) → **0 fetches** — slim list from DB only
- `GET /contracts/:id/events/:id` (detail, **new endpoint**) → **1 fetch** — `parsed_ledger_{ledger_sequence}.json`
- `GET /transactions/:hash` (detail) → **1 fetch** (same file)

**API contract change:** event list returns slim data (contract_id, event_type, topic0,
timestamp). Full topics + data available in per-event detail or transaction detail.

**New detail endpoint:** `GET /contracts/:id/events/:event_id`

```
1. DB: SELECT ledger_sequence, transaction_id, event_index FROM soroban_events WHERE id = :event_id
2. DB: SELECT hash FROM transactions WHERE id = transaction_id
3. S3: GET parsed_ledger_{ledger_sequence}.json → transactions[hash].events[event_index]
```

Bridge columns (`ledger_sequence`, `transaction_id`, `event_index`) remain in DB.

**Note on partition scan:** Lookup by `id` alone cannot prune partitions (partition key is
`created_at`). PostgreSQL performs per-partition index scans — at tens of partitions this is
<10ms. If partition count grows significantly (100+, i.e. ~10 years of data), consider
encoding `created_at` in the client-facing identifier or adding a lookup table.

#### 6. `soroban_invocations` — OFFLOAD `function_args`+`return_value`

Relatively small now (6.9K rows, 3.8 MB on 100 ledgers) but grows with Soroban adoption.

**Removed from DB:**

| Column                  | Avg/row            | Reason            |
| ----------------------- | ------------------ | ----------------- |
| `function_args` (JSONB) | 164 B (max 1.7 KB) | Only detail views |
| `return_value` (JSONB)  | 48 B (max 7.3 KB)  | Only detail views |

**New schema:**

```sql
soroban_invocations (
  id                BIGSERIAL,
  transaction_id    BIGINT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
  contract_id       VARCHAR(56),
  caller_account    VARCHAR(69),             -- 69: muxed M-addresses (SEP-0023)
  function_name     VARCHAR(100) NOT NULL,
  successful        BOOLEAN NOT NULL,
  invocation_index  SMALLINT NOT NULL DEFAULT 0,
  ledger_sequence   BIGINT NOT NULL,   -- denormalized bridge to S3 (no FK to ledgers — parallel backfill safety)
  created_at        TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (id, created_at),
  UNIQUE (transaction_id, invocation_index, created_at)
) PARTITION BY RANGE (created_at);
-- Dedup: ON CONFLICT (transaction_id, invocation_index, created_at) DO NOTHING
-- Note: created_at in UNIQUE is required by PostgreSQL (partition key must be in all unique constraints)
-- Column-list form required: ON CONFLICT ON CONSTRAINT <name> does not work on partitioned tables
```

**S3:** `invocations[].function_args` and `invocations[].return_value` included in
`parsed_ledger_{sequence}.json` → `transactions[].invocations[]`.

**Reduction:** 3.8 MB → ~2.4 MB on 100 ledgers. Savings: ~1.4 MB (small now, significant
at mainnet Soroban scale).

**S3 fetch count per endpoint:**

- `GET /contracts/:id/invocations` (list) → **0 fetches** — slim list from DB only
- `GET /contracts/:id/invocations/:id` (detail, **new endpoint**) → **1 fetch**
- `GET /transactions/:hash` (detail) → **1 fetch** (same file)

**New detail endpoint:** `GET /contracts/:id/invocations/:invocation_id`

```
1. DB: SELECT ledger_sequence, transaction_id, invocation_index FROM soroban_invocations WHERE id = :invocation_id
2. DB: SELECT hash FROM transactions WHERE id = transaction_id
3. S3: GET parsed_ledger_{ledger_sequence}.json → transactions[hash].invocations[invocation_index]
```

Bridge columns (`ledger_sequence`, `transaction_id`, `invocation_index`) remain in DB.

**Note on partition scan:** Same as events — lookup by `id` alone scans all partitions.
Acceptable at current scale; same mitigation path if partition count grows.

**API contract change:** invocation list returns slim data (contract_id, caller_account,
function_name, successful, timestamp). Full function_args + return_value in per-invocation
detail or transaction detail.

#### 7. `accounts` — NORMALIZE `balances` JSONB + balance history (no S3 needed)

Current schema stores balances as a JSONB array with watermark upsert (overwrite on each
ledger). Measured: 90% of accounts have a single native XLM entry (61 B), <1% have >1 KB
(40-60 trustlines). Total: 1.9 MB heavy on 13.7K accounts.

Instead of offloading to S3, normalize the JSONB into a proper relational table with
**insert-only balance history** (cumulative inserts instead of upserts). This is more
appropriate because:

- `accounts` is **mutable** — S3 PUT per upsert adds cost/complexity
- The table is small — S3 savings would be minimal
- Normalized data enables SQL filtering by asset (impossible with JSONB without GIN)
- Insert-only balance history is a standard block explorer feature (Etherscan has
  Account Balance Checker for historical ETH/token balances at any block number)
- Without history, reconstructing past balances requires full chain re-index

**Removed from `accounts`:**

| Column             | Reason                                          |
| ------------------ | ----------------------------------------------- |
| `balances` (JSONB) | Normalized into `account_balances` with history |

**New schemas (Variant B):**

```sql
accounts (
  account_id        VARCHAR(69) PRIMARY KEY,  -- 69: muxed M-addresses (SEP-0023)
  first_seen_ledger BIGINT NOT NULL,
  last_seen_ledger  BIGINT NOT NULL,
  sequence_number   BIGINT NOT NULL,
  home_domain       VARCHAR(256)
)
-- ↑ upsert (mutable, lightweight — zero JSONB)

account_balances (
  account_id      VARCHAR(69) NOT NULL,     -- 69: muxed M-addresses (SEP-0023)
  ledger_sequence BIGINT NOT NULL,
  asset_type      VARCHAR(20) NOT NULL,
  asset_code      VARCHAR(12) NOT NULL DEFAULT '',
  issuer          VARCHAR(69) NOT NULL DEFAULT '',  -- 69: issuer is always G-address (56) but VARCHAR(69) for consistency
  balance         NUMERIC(39,0) NOT NULL,   -- raw i128 integer; format via contract's decimals()
  PRIMARY KEY (account_id, ledger_sequence, asset_type, asset_code, issuer)
)
-- ↑ insert-only (cumulative balance history per asset per ledger)
-- Dedup: ON CONFLICT (account_id, ledger_sequence, asset_type, asset_code, issuer) DO NOTHING
-- Native XLM: asset_code = '', issuer = '' (empty strings, not NULL)
-- No FK to accounts(account_id) — intentional: during parallel backfill, balance rows
-- may arrive before the corresponding account row (different workers, different ledger ranges).
```

**Example data:**

```
-- accounts (1 row per account, upsert):
account_id | first_seen | last_seen | sequence_number | home_domain
GABC...    | 100        | 300       | 1004            | example.com

-- account_balances (many rows, insert-only):
account_id | ledger | asset_type | asset_code | issuer  | balance
GABC...    | 100    | native     |            |         | 500.00
GABC...    | 150    | native     |            |         | 450.00
GABC...    | 150    | credit_4   | USDC       | GA5Z... | 100.00
GABC...    | 200    | native     |            |         | 300.00
GABC...    | 200    | credit_4   | USDC       | GA5Z... | 100.00
```

**Variant A considered and rejected:**

Variant A proposed full cumulative history for `accounts` too — an `account_states` table
with insert-only rows tracking `sequence_number` and `home_domain` per ledger (full
account snapshot at every change). Rejected because:

- `sequence_number` (nonce) increments by 1 on every transaction — hundreds of thousands
  of rows where the only change is nonce+1
- `home_domain` changes extremely rarely (once in account lifetime, if ever)
- No block explorer (including Etherscan) offers historical nonce/home_domain lookup —
  it's a niche debug use case
- Extra ~6 MB per 100 ledgers for data nobody queries

Variant B keeps `accounts` as lightweight upsert (current state only) and puts cumulative
history only on `account_balances` where it provides real user value.

**Size estimate:**

|                    | On 100 ledgers     | Mainnet (estimate)         |
| ------------------ | ------------------ | -------------------------- |
| `accounts`         | 13.7K rows, ~1 MB  | ~few million rows, ~200 MB |
| `account_balances` | ~334K rows, ~27 MB | few-tens of GB             |

**No S3 involvement.** All data remains in DB, fully relational and lightweight.

**API flow (0 S3 fetches for all endpoints):**

- `GET /accounts/:id` → DB: accounts + account_balances (latest per asset)
- `GET /accounts/:id/transactions` → DB: transactions WHERE source_account = :id
- `GET /network/stats` → DB: COUNT(\*) FROM accounts
- `GET /search` → DB: prefix match on account_id
- Future: balance history → DB: account_balances ORDER BY ledger_sequence
- Future: balance at block X → DB: account_balances WHERE ledger_sequence <= X

**Additional benefits:**

- "Top holders of USDC" queries become trivial SQL
- `GET /tokens/:id` holder_count can be derived from `account_balances`
- Insert-only = no lock contention, no race conditions during parallel backfill
- No watermark needed on balances — every state is preserved

#### 8. `tokens` — OFFLOAD `metadata` to S3

Small, immutable table (ON CONFLICT DO NOTHING). Max tens of thousands of rows on mainnet.
`metadata` JSONB is currently always NULL (not even in INSERT column list), but planned
for future use (logo, description, links).

**Removed from DB:**

| Column             | Reason                                                                |
| ------------------ | --------------------------------------------------------------------- |
| `metadata` (JSONB) | Only token detail view; currently unused; moved to S3 for consistency |

**New schema:**

```sql
tokens (
  id               SERIAL PRIMARY KEY,
  asset_type       VARCHAR(20) NOT NULL,
  asset_code       VARCHAR(12),
  issuer_address   VARCHAR(56),
  contract_id      VARCHAR(56),
  name             VARCHAR(256),
  total_supply     NUMERIC(39,0),            -- raw i128 integer; format via contract's decimals()
  holder_count     INTEGER,
  metadata_ledger  BIGINT          -- bridge to S3: parsed_ledger_{metadata_ledger}.json
)
-- Note: total_supply and holder_count are currently always NULL — no write path exists.
-- The table is insert-once (ON CONFLICT DO NOTHING), so these cannot be set after insert.
-- A separate UPDATE mechanism will be needed when these values are populated (e.g.,
-- holder_count derived from account_balances, total_supply from ledger entries).
--
-- Unique constraints (dedup targets for ON CONFLICT DO NOTHING):
-- idx_tokens_classic: UNIQUE (asset_code, issuer_address) WHERE asset_type IN ('classic', 'sac')
-- idx_tokens_soroban: UNIQUE (contract_id) WHERE asset_type = 'soroban'
-- idx_tokens_sac:     UNIQUE (contract_id) WHERE asset_type = 'sac'
```

**S3:** Token metadata included in `parsed_ledger_{sequence}.json` → `token_metadata[]`
(when metadata exists). Bridge column `metadata_ledger` needed in `tokens` to locate
the correct S3 file.

**API flow:**

- `GET /tokens` (list) → **0 fetches** — DB only, list doesn't need metadata
- `GET /tokens/:id` (detail) → **1 fetch** — `parsed_ledger_{metadata_ledger}.json`
- `GET /tokens/:id/transactions` → **0 fetches** — DB JOIN
- `GET /search` → **0 fetches** — DB prefix match on asset_code

#### 9. `nfts` — SPLIT into `nfts` + `nft_ownership` (no S3 needed)

Table is **mutable** (watermark upsert on owner_account at every transfer) and **small**
(912 rows, 648 KB on 100 ledgers; max tens of thousands on mainnet).

S3 is not appropriate for mutable, small tables. Instead, split into two tables:

- `nfts` — NFT identity and metadata (insert-once, mostly immutable)
- `nft_ownership` — ownership history (insert-only, cumulative)

This follows the same pattern as `accounts` + `account_balances`: separate immutable
entity data from cumulative change history. Enables `GET /nfts/:id/transfers` without
JOINing `soroban_events` (which has topics/data on S3).

`metadata` stays as JSONB in DB because NFT metadata has no standard schema in Stellar
(SEP-0050 leaves structure to contract developers). Each contract defines arbitrary
attributes, potentially nested. Normalization (EAV pattern) would lose types and
nested structures. Table is small — max few MB on mainnet.

**Removed from original `nfts`:**

| Column             | Reason                                                               |
| ------------------ | -------------------------------------------------------------------- |
| `owner_account`    | Moved to `nft_ownership` (history instead of latest-only)            |
| `last_seen_ledger` | No longer needed — `nfts` is insert-once, history in `nft_ownership` |

**Added:**

| Column                 | Reason                                                                                      |
| ---------------------- | ------------------------------------------------------------------------------------------- |
| `id` (SERIAL PK)       | Surrogate key — `nft_ownership` FK is 4 bytes instead of 312 bytes (contract_id + token_id) |
| `current_owner`        | Denormalized from latest `nft_ownership` — avoids LATERAL JOIN on list queries              |
| `current_owner_ledger` | Watermark guard for `current_owner` — prevents stale parallel-backfill overwrites           |

**New schemas:**

```sql
nfts (
  id                    SERIAL PRIMARY KEY,
  contract_id           VARCHAR(56) NOT NULL,
  token_id              VARCHAR(256) NOT NULL,
  collection_name       VARCHAR(256),
  name                  VARCHAR(256),
  media_url             TEXT,
  metadata              JSONB,
  minted_at_ledger      BIGINT,
  current_owner         VARCHAR(69),             -- 69: muxed M-addresses (SEP-0023)
  current_owner_ledger  BIGINT,
  UNIQUE (contract_id, token_id)
)
-- Insert-once. name/media_url/metadata: COALESCE (NULL → value, max once).
-- current_owner + current_owner_ledger: updated on every INSERT to nft_ownership.
-- Watermark guard: UPDATE nfts SET current_owner = $1, current_owner_ledger = $2
--   WHERE id = $3 AND (current_owner_ledger IS NULL OR current_owner_ledger < $2)
-- This prevents stale parallel-backfill workers from overwriting newer ownership.

-- Indexes:
-- PK: id
-- UNIQUE: (contract_id, token_id)
-- idx_nfts_collection: (contract_id, collection_name)
-- idx_nfts_owner: (current_owner)

nft_ownership (
  nft_id          INTEGER NOT NULL REFERENCES nfts(id) ON DELETE CASCADE,
  transaction_id  BIGINT NOT NULL,          -- no FK to transactions — parallel backfill safety (same pattern as soroban_events)
  owner_account   VARCHAR(69),            -- NULL on burn; 69: muxed M-addresses (SEP-0023)
  event_type      VARCHAR(20) NOT NULL,   -- 'mint', 'transfer', 'burn'
  ledger_sequence BIGINT NOT NULL,
  event_order     SMALLINT NOT NULL,       -- tiebreaker: event index within the ledger
  created_at      TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (nft_id, ledger_sequence, event_order)
)
-- Insert-only. Full ownership history.
-- Dedup: ON CONFLICT (nft_id, ledger_sequence, event_order) DO NOTHING
-- event_order: disambiguates multiple ownership changes in the same ledger
-- (e.g., mint + transfer in one tx, or two transfers in separate txs within one ledger).

-- Indexes:
-- PK: (nft_id, ledger_sequence, event_order) — covers "history" and "latest" queries
-- idx_nft_ownership_owner: (owner_account) — "show NFTs owned by account X"
```

**Write path:**

1. Mint: INSERT into `nfts` (with current_owner, current_owner_ledger) + INSERT into `nft_ownership`
2. Transfer/burn: INSERT into `nft_ownership` (with event_order) + UPDATE `nfts SET current_owner = ?, current_owner_ledger = ? WHERE id = ? AND (current_owner_ledger IS NULL OR current_owner_ledger < ?)`

**API flow (0 S3 fetches for all endpoints):**

- `GET /nfts` (list) → DB: `SELECT * FROM nfts` (current_owner already in table)
- `GET /nfts/:id` (detail) → DB: `nfts WHERE id = ?`
- `GET /nfts/:id/transfers` → DB: `nft_ownership JOIN transactions ON transaction_id = transactions.id WHERE nft_id = ? ORDER BY ledger_sequence`
- Future: NFT state at block X → DB: `nft_ownership WHERE nft_id = ? AND ledger_sequence <= X ORDER BY ledger_sequence DESC, event_order DESC LIMIT 1`

#### 10. `liquidity_pools` — NORMALIZE JSONB to relational columns, remove mutable state

Table is small (979 rows, 704 KB). Three JSONB columns (`asset_a`, `asset_b`, `reserves`)
all have known, fixed structures. Normalize to relational columns.

Mutable state (`reserves`, `total_shares`, `tvl`) is already tracked in
`liquidity_pool_snapshots` (insert-only history). Removing these from `liquidity_pools`
eliminates dupli­cation — current state is always the latest snapshot.

`current_*` columns (denormalized shortcut) intentionally omitted. At ~thousands of pools,
LATERAL JOIN to snapshots is fast enough. If `filter[min_tvl]` becomes slow, a migration
can add `current_tvl` derived from snapshots — no re-index needed.

**Removed from DB:**

| Column                         | Reason                                                           |
| ------------------------------ | ---------------------------------------------------------------- |
| `asset_a` (JSONB)              | Normalized into `asset_a_type`, `asset_a_code`, `asset_a_issuer` |
| `asset_b` (JSONB)              | Normalized into `asset_b_type`, `asset_b_code`, `asset_b_issuer` |
| `reserves` (JSONB)             | Redundant — already in snapshots history                         |
| `total_shares` (NUMERIC)       | Redundant — already in snapshots history                         |
| `tvl` (NUMERIC)                | Redundant — already in snapshots history                         |
| `last_updated_ledger` (BIGINT) | No longer needed — table is now immutable                        |

**New schema:**

```sql
liquidity_pools (
  pool_id           VARCHAR(64) PRIMARY KEY,
  asset_a_type      VARCHAR(20) NOT NULL,
  asset_a_code      VARCHAR(12),
  asset_a_issuer    VARCHAR(56),
  asset_b_type      VARCHAR(20) NOT NULL,
  asset_b_code      VARCHAR(12),
  asset_b_issuer    VARCHAR(56),
  fee_bps           INTEGER NOT NULL,
  created_at_ledger BIGINT NOT NULL
)
-- Immutable, insert-once. Identity and static config only.
-- Current reserves/tvl/shares → latest snapshot.
```

**API flow (0 S3 fetches):**

- `GET /liquidity-pools` (list) → DB: `liquidity_pools JOIN LATERAL snapshots (latest)`
- `GET /liquidity-pools/:id` (detail) → DB: `liquidity_pools + snapshots (latest)`
- `GET /liquidity-pools/:id/chart` → DB: `liquidity_pool_snapshots`
- `filter[assets]` → DB: `WHERE asset_a_code = ? OR asset_b_code = ?` (B-tree index)
  Note: multi-asset filter semantics (single asset OR vs asset pair AND) to be defined
  at implementation time. Schema supports both — columns are in DB.
- `filter[min_tvl]` → DB: LATERAL JOIN snapshots → WHERE tvl > ?

#### 11. `liquidity_pool_snapshots` — NORMALIZE `reserves` JSONB to relational columns

Insert-only, monthly partitioned history table. Only change: `reserves` JSONB (`{a, b}`)
normalized to two NUMERIC columns.

**New schema:**

```sql
liquidity_pool_snapshots (
  id               BIGSERIAL,
  pool_id          VARCHAR(64) NOT NULL REFERENCES liquidity_pools(pool_id),
  ledger_sequence  BIGINT NOT NULL,
  created_at       TIMESTAMPTZ NOT NULL,
  reserve_a        NUMERIC(39,0) NOT NULL,   -- raw i128 integer; format via contract's decimals()
  reserve_b        NUMERIC(39,0) NOT NULL,   -- raw i128 integer; format via contract's decimals()
  total_shares     NUMERIC NOT NULL,
  tvl              NUMERIC,
  volume           NUMERIC,
  fee_revenue      NUMERIC,
  PRIMARY KEY (id, created_at),
  UNIQUE (pool_id, ledger_sequence, created_at)
) PARTITION BY RANGE (created_at);
-- Insert-only. Full history of pool state per ledger change.
```

**API flow (0 S3 fetches):**

- `GET /liquidity-pools/:id/chart` → DB: `SELECT reserve_a, reserve_b, tvl, volume, fee_revenue FROM liquidity_pool_snapshots WHERE pool_id = ? AND created_at BETWEEN ? AND ?`

#### 12. `wasm_interface_metadata` — OFFLOAD `metadata` JSONB to S3, keep only `name`

Staging table that bridges Soroban's 2-ledger deploy pattern: WASM uploaded in ledger N
(producing interface metadata), contract deployed in ledger N+k. In the current schema,
full metadata JSONB (10-100 KB) is staged here and later JOINed into
`soroban_contracts.metadata`.

Since `soroban_contracts.metadata` moved to S3 (section 4), this staging table no longer
needs full metadata either. Only `name` is needed — to populate `soroban_contracts.name`
(used for `search_vector` generation).

**Removed from DB:**

| Column             | Reason                              |
| ------------------ | ----------------------------------- |
| `metadata` (JSONB) | Full interface metadata moved to S3 |

**Added:**

| Column               | Reason                                                                                                        |
| -------------------- | ------------------------------------------------------------------------------------------------------------- |
| `name`               | Extracted from metadata for `soroban_contracts.name` population at deploy time                                |
| `uploaded_at_ledger` | Bridge to S3: WASM function signatures are in `parsed_ledger_{uploaded_at_ledger}.json`                       |
| `contract_type`      | Pre-computed by parser from WASM function signatures (task 0118); propagated to `soroban_contracts` at deploy |

**New schema:**

```sql
wasm_interface_metadata (
  wasm_hash          VARCHAR(64) PRIMARY KEY,
  name               VARCHAR(256),
  uploaded_at_ledger BIGINT NOT NULL,
  contract_type      VARCHAR(50) NOT NULL DEFAULT 'other'
)
-- Lightweight staging. Full metadata on S3: parsed_ledger_{uploaded_at_ledger}.json → wasm_uploads[]
-- Used by deploy step to populate soroban_contracts.name and contract_type
-- contract_type: 'nft', 'fungible', or 'other' — classified by parser at WASM upload time
```

**S3:** WASM function signatures included in `parsed_ledger_{uploaded_at_ledger}.json`
→ `wasm_uploads[]` (keyed by `wasm_hash`).

**Ingestion flow:**

1. WASM upload (ledger N):
   - **Parse phase:** parse WASM → extract function signatures → classify
     `contract_type` (`'nft'` if `owner_of`/`token_uri`, `'fungible'` if
     `decimals`+`balance→i128`, else `'other'`) → extract `name`
   - **Persist phase (parallel):**
     - S3: full WASM metadata included in `parsed_ledger_{N}.json` → `wasm_uploads[]`
     - DB: INSERT staging (`wasm_hash`, `name`, `uploaded_at_ledger = N`, `contract_type`)
   - `contract_type` is a ready-to-write value computed by the parser, not by the
     DB persist step
2. Contract deploy (ledger N+k):
   - **Persist phase:** JOIN `wasm_interface_metadata` → set `soroban_contracts.name`
     and `contract_type` from the pre-computed classification

**Parallel backfill caveat:** With parallel backfill, a worker may process the deploy
ledger (N+k) before another worker processes the WASM upload ledger (N). In that case
`wasm_interface_metadata` has no row yet — `contract_type` stays `'other'` in
`soroban_contracts`. The existing catch-up path (`update_contract_interfaces_by_wasm_hash`)
currently updates only `metadata` JSONB, not `contract_type`. Before enabling parallel
backfill, this function must be extended to also propagate `contract_type`, or a
post-backfill reconciliation UPDATE must be run. Not an issue for sequential backfill
(ledgers processed in order guarantee WASM upload before deploy).

**API flow:**

- `GET /contracts/:id/interface` → DB: `wasm_hash` + `uploaded_at_ledger` from
  `wasm_interface_metadata` → **1 S3 fetch**: `parsed_ledger_{uploaded_at_ledger}.json`
  → `wasm_uploads[wasm_hash]`
- `GET /contracts/:id` (detail) → DB + **1 S3 fetch** `parsed_ledger_{deployed_at_ledger}.json`
- `GET /search` → DB: `search_vector` (name in `soroban_contracts`) → **0 fetches**

---

## Rationale

### Core principle: DB = lightweight index, S3 = full parsed data

RDS storage is ~10x more expensive than S3. Measured on 100 ledgers: 87% of DB size is
JSONB/TEXT blobs used only in detail views, never for filtering or pagination. Moving these
to S3 reduces projected mainnet DB from 2-5 TB to ~200-650 GB.

### When to use S3 vs DB vs normalization

| Data characteristic                   | Approach                      | Examples                                   |
| ------------------------------------- | ----------------------------- | ------------------------------------------ |
| Immutable, heavy, high-volume         | **S3**                        | XDRs, operation details, event payloads    |
| Mutable, small, needed for filtering  | **DB (relational)**           | accounts, nfts, liquidity pools            |
| JSONB with known fixed structure      | **Normalize to columns**      | asset_a/b, reserves, balances              |
| JSONB with unknown/variable structure | **Keep as JSONB in DB**       | NFT metadata (no standard schema)          |
| Mutable state with historical value   | **Insert-only history table** | account_balances, nft_ownership, snapshots |

### S3 fetch budget per endpoint

Every detail endpoint requires **at most 1 S3 fetch** (`parsed_ledger_{sequence}.json`).
List endpoints require **0 S3 fetches** — served entirely from DB.
Bridge column `ledger_sequence` (or `deployed_at_ledger`, `uploaded_at_ledger`,
`metadata_ledger`) in every table points to the correct S3 file.

---

## Alternatives Considered

### Alternative 1: Keep all data in RDS

**Description:** Current approach — all parsed XDR data stored in PostgreSQL.

**Pros:**

- Simple architecture, single data source
- No S3 read latency on detail endpoints

**Cons:**

- Projected 2-5 TB at mainnet scale — expensive on RDS
- 87% of storage is heavy blobs used only in detail views

**Decision:** REJECTED — cost at mainnet scale is prohibitive.

### Alternative 2: S3 for mutable tables (accounts, nfts, liquidity pools)

**Description:** Move all heavy data to S3, including mutable tables.

**Pros:**

- Maximum RDS size reduction

**Cons:**

- S3 PUT on every upsert (accounts change every ledger)
- Race conditions on S3 during parallel backfill
- Complexity for minimal savings (mutable tables are small)

**Decision:** REJECTED — mutable tables are small, normalization achieves the same
lightweight goal without S3 complexity.

### Alternative 3: Staging table for NFT/event data

**Description:** Insert unknowns into staging tables, resolve after backfill.

**Pros:**

- Clean separation of unresolved vs resolved data

**Cons:**

- Extra tables, resolve logic, retention policies

**Decision:** REJECTED — post-backfill cleanup DELETE is simpler.

---

## Consequences

### Positive

- ~87% reduction in RDS storage (projected 2-5 TB → 200-650 GB at mainnet)
- S3 storage is ~10x cheaper than RDS per GB
- List endpoints faster (smaller tables, no JSONB parsing)
- Insert-only history tables (account_balances, nft_ownership) enable historical queries
  (balance at block X, NFT ownership at block X) without chain re-index
- Zero JSONB in normalized tables (liquidity_pools, account_balances) enables SQL filtering
  that was impossible with JSONB
- Insert-only tables eliminate lock contention and race conditions during parallel backfill

### Negative

- Detail endpoints have added S3 latency (~50-100ms per GET)
- Two data sources (DB + S3) increase operational complexity
- Event/invocation list endpoints become slim — require new per-item detail endpoints
- `current_owner` in nfts is denormalized — must be kept in sync on every transfer
- `filter[min_tvl]` uses LATERAL JOIN instead of direct WHERE — may need denormalized
  `current_tvl` column later if pool count grows significantly

### Note: per-block history coverage

The schema preserves per-block history for all data with historical user value:

- **Full per-block history:** `account_balances` (balance at any ledger), `nft_ownership`
  (owner at any ledger), `liquidity_pool_snapshots` (pool state at any ledger)
- **Immutable (no history needed):** `ledgers`, `transactions`, `operations`,
  `soroban_events`, `soroban_invocations`, `liquidity_pools`, `tokens`,
  `wasm_interface_metadata` — these do not change after insert

Two tables do **not** preserve per-block history:

**`accounts`** — `sequence_number` and `home_domain` are overwritten on each upsert
(watermark-guarded). Previous values are lost. This is intentional:

- `sequence_number` (nonce) increments on every transaction — storing history would
  produce millions of rows where the only change is nonce+1
- `home_domain` changes at most once in an account's lifetime
- No block explorer offers historical nonce/home_domain lookup — it is a niche debug
  use case with no user value
- Balance history (the data users actually want) is fully preserved in `account_balances`

**`soroban_contracts`** — columns like `contract_type`, `name`, `wasm_hash` are upserted
via COALESCE (NULL → value). However, this is not a real loss of history because:

- No column is ever overwritten with a different value — the pattern is always
  NULL → value → same value forever (progressive fill of a stub row)
- Soroban contracts are immutable on-chain after deployment (same WASM, same deployer,
  same ledger)
- `is_sac` uses OR logic (sticky TRUE → never reverts)
- There is no meaningful "previous state" to track — the only transition is from
  unknown (NULL) to known (value)

If per-block history for these tables becomes necessary in the future, insert-only
history tables (same pattern as `account_balances`) can be added via migration without
affecting existing data.

---

## References

- [Database Audit](../../docs/database-audit-first-implementation.md) — full table-by-table audit
- [Backend Overview](../../docs/architecture/backend/backend-overview.md) — endpoint inventory
- [Technical Design General Overview](../../docs/architecture/technical-design-general-overview.md) — endpoint specs
- [SEP-0050: Non-Fungible Tokens](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0050.md) — NFT metadata standard
- [Etherscan Account Balance Checker](https://etherscan.io/balancecheck-tool) — historical balance feature reference
