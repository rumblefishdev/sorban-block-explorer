---
id: '0014'
title: 'Schema fixes — Stellar/XDR compliance and history correctness'
status: proposed
deciders: [fmazur]
related_tasks: []
related_adrs: ['0011', '0012', '0013']
tags: [database, schema, stellar, xdr, muxed, memo, soroban-events, search]
links: []
history:
  - date: 2026-04-19
    status: proposed
    who: fmazur
    note: 'ADR created — corrective revision of ADR 0013 after critical review'
---

# ADR 0014: Schema fixes — Stellar/XDR compliance and history correctness

**Related:**

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0012: Lightweight bridge DB schema revision](0012_lightweight-bridge-db-schema-revision.md)
- [ADR 0013: Sequential ingest schema with full FK integrity](0013_sequential-ingest-full-fk-schema.md)

---

## Status

`proposed` — corrective revision on top of ADR 0013. Does not supersede 0013 in full;
changes listed here are deltas applied to the ADR 0013 model. Table shapes,
partitioning, and FK graph from ADR 0013 are preserved unless this ADR explicitly
changes them.

---

## Context

ADR 0013 defined the full FK graph for sequential ingest. A subsequent critical
review surfaced seven concrete correctness issues that this ADR must resolve without
enlarging the model:

1. **Muxed address identity is undefined.** `VARCHAR(69)` on `accounts.account_id`
   with FKs from `source_account`, `from_account`, etc. forces every muxed
   sub-identifier to produce its own `accounts` row. Balances, transaction history,
   and `/accounts/:id` queries fragment across M-form variants of the same
   ed25519 public key. Directly contradicts SEP-0023 semantics, which treats muxed
   identifiers as payment-routing hints on top of a single underlying account.

2. **`soroban_events.topic0 VARCHAR(32)` truncates legal ScVal values.** Symbol is
   at most 32 bytes, but CAP-67 topics also legally contain `Address` (up to 69
   hex chars), bytes, u256, integers, or string — stringified representations
   exceed 32 regularly.

3. **Global uniqueness of `transactions.hash` is not enforced on the partitioned
   table.** Postgres requires the partition key to be included in every unique
   constraint on a partitioned table — `UNIQUE (hash, created_at)` is only per-row
   unique, not globally unique. Replay bugs can produce silent duplicates.

4. **`memo` as `VARCHAR(128)`** forces UTF-8 encoding. Stellar `memo_text` is 28
   binary bytes (not guaranteed UTF-8); `memo_hash`/`memo_return` are 32 raw bytes.
   VARCHAR storage mangles non-UTF-8 payloads.

5. **Enum-like `VARCHAR` columns lack `CHECK`** — `memo_type`, `event_type`,
   `transfer_type`, `role`, `contract_type`, `asset_type` (on balance tables) —
   all backed by finite XDR/SEP enums, none enforced at the database level.

6. **Search/prefix lookup paths have no supporting indexes.** Prefix `LIKE 'GABC%'`
   on `accounts.account_id` or `transactions.hash` does a sequential scan because
   default B-tree uses locale collation. The `/search` endpoint degrades under
   load.

7. **History-vs-S3 boundary for "replayable ledger state" needs an explicit
   contract.** ADR 0013 leaves this implicit; ingest contract must be spelled out
   so the parser cannot silently drift into producing rows that break historical
   queries.

This ADR resolves all seven without adding new tables. The principle "DB as
lightweight bridge, S3 as heavy payload" is reaffirmed and every proposed change
honors it.

---

## Decision

### Summary of decisions

- **Canonical account identity = G-address** (`VARCHAR(56)`, ed25519 public key).
  Muxed M-form is stored only in auxiliary `*_muxed VARCHAR(69) NULL` columns on
  sites where routing hints matter. FKs point at G-address. All aggregation,
  history, and endpoints operate on G.
- **`soroban_events.topic0` is `TEXT`.** No truncation. Parser produces a canonical
  stringification of the ScVal (`Symbol` and `String` as raw text; `Address`,
  `bytes`, integers as their canonical hex/decimal representation). B-tree index
  on `topic0` unchanged — Postgres indexes arbitrary-length TEXT up to ~2.7 KB
  per entry, and all realistic topic0 values fit well under that bound.
- **Transaction hash global uniqueness is enforced by the existing
  `UNIQUE (hash, created_at)` in combination with a parser invariant:
  `transactions.created_at = ledgers.closed_at` for that `ledger_sequence`.**
  Stellar transaction hash is deterministic over `TransactionEnvelope +
network_passphrase` (SEP-0005 / Stellar core). An envelope is applied in exactly
  one ledger in any valid chain state, so `(hash, created_at)` is globally unique
  iff `created_at` is deterministic per hash — which the parser invariant
  guarantees. Re-ingest of ledger N deletes rows for `ledger_sequence = N`
  (cascade) and re-inserts with the same `created_at` — `ON CONFLICT (hash,
created_at) DO NOTHING` is idempotent. **No new tables.**
- **`memo` is stored as `BYTEA`**, with `memo_type` as the discriminator. API
  layer interprets bytes according to `memo_type` (UTF-8 attempt for `text`,
  base64 fallback; u64 decode for `id`; hex for `hash`/`return`).
- **`CHECK` constraints** are added to six columns where the value space is a
  finite enum fixed by protocol (XDR, SEP-41, SEP-50, CAP-67). No others.
- **Prefix-scan indexes** (`text_pattern_ops`) are added to the four primary
  lookup keys (`transactions.hash`, `accounts.account_id`,
  `soroban_contracts.contract_id`, `liquidity_pools.pool_id`). Two trigram
  indexes (`gin_trgm_ops`) are added for fuzzy name search
  (`nfts.name`, `tokens.asset_code`). No other search infrastructure.
- **Ledger-linked history contract** is made explicit in this ADR: which tables
  reconstruct state from DB alone, which require DB + one S3 fetch, and the
  parser invariants that make both paths correct.

### Canonical account identity (fix #1)

**Decision:**

- `accounts.account_id` is narrowed from `VARCHAR(69)` to **`VARCHAR(56)`**.
  The primary key is the G-address (ed25519 public key, StrKey-encoded).
- All FK-bearing account references across the schema — `source_account`,
  `destination`, `caller_account`, `owner_account` (on nfts), `deployer_account`,
  `fee_account`, `from_account`, `to_account`, `asset_issuer`, `issuer_address`,
  `current_owner` — are **`VARCHAR(56)`** and reference `accounts(account_id)`.
- Sites where the original M-form carries routing information keep an auxiliary
  nullable column `*_muxed VARCHAR(69)`, populated only when the XDR value was a
  muxed M-address. Columns added:
  - `transactions.source_account_muxed VARCHAR(69)`
  - `transactions.fee_account_muxed VARCHAR(69)`
  - `operations.source_account_muxed VARCHAR(69)`
  - `operations.destination_muxed VARCHAR(69)`
  - `soroban_invocations.caller_account_muxed VARCHAR(69)`
  - `token_transfers.from_account_muxed VARCHAR(69)`
  - `token_transfers.to_account_muxed VARCHAR(69)`
- `*_muxed` columns are **not indexed** and **not referenced by FK**. They exist
  only to preserve the original XDR byte sequence for advanced/detail views and
  debug. List filters, aggregation, joins, and history all operate on the G-form
  column.
- Parser invariant: if the XDR value is M-form, the parser computes the
  underlying G-address (trivial — muxed M-address carries the G in its payload
  per SEP-0023) and writes G to the FK column, preserving the full M-form in
  `*_muxed`. If the XDR is already G-form, `*_muxed` is `NULL`.

**Out of scope for this ADR:** whether API responses should surface muxed M-form
as `account_muxed` field alongside `account`. That is a separate API-layer
decision — the schema now supports either choice.

### `soroban_events.topic0` (fix #2)

**Decision:**

- `topic0` column type changes from `VARCHAR(32)` to **`TEXT`**.
- Parser canonicalization rules, normative for this column:
  - `Symbol` → raw symbol text (e.g. `"transfer"`).
  - `String` → raw string text.
  - `Address` → StrKey-encoded form (56 or 69 chars).
  - `Bytes` → lowercase hex without `0x` prefix.
  - `u32 | i32 | u64 | i64` → decimal string.
  - `u128 | i128 | u256 | i256` → decimal string.
  - `Bool` → `"true"` / `"false"`.
  - Any other ScVal type → base64-encoded XDR (canonical fallback, never lossy).
- The existing B-tree index `idx_events_topic0 (contract_id, topic0, created_at
DESC) WHERE topic0 IS NOT NULL` is unchanged; Postgres B-tree indexes TEXT up
  to ~2704 bytes per entry, orders of magnitude above any realistic topic0.

### Transaction hash uniqueness (fix #3)

**Decision:**

- No new table. Existing constraint `UNIQUE (hash, created_at)` on
  `transactions` is retained.
- **Parser invariant (normative):** every `transactions` row for a given
  `ledger_sequence` is inserted with `created_at = ledgers.closed_at` for that
  ledger. No other value is legal. This invariant is documented in the indexer
  code as a parse-phase assertion.
- **Why this is sufficient:** Stellar transaction hash is deterministic over
  `TransactionEnvelope + network_passphrase` (see Stellar core,
  `Transaction.hash()`). A given envelope is valid in exactly one ledger in any
  correct protocol execution (sequence number consumed, tx slot committed). Two
  rows with the same `hash` can only differ in `created_at` if the parser
  violates the invariant — which would be a parser bug surfaced by downstream
  data checks, not a protocol ambiguity.
- **Re-ingest protocol (normative):** to re-ingest ledger N, run
  `DELETE FROM transactions WHERE ledger_sequence = N`. Cascade removes children
  in all FK-attached tables. Then re-insert with `ON CONFLICT (hash, created_at)
DO NOTHING` semantics (no-op since cascade already cleared the slot). The
  invariant guarantees the same `created_at` is produced, so any surviving
  lookup uses the correct composite.
- **Detection of invariant violation:** a monitoring query
  `SELECT hash, COUNT(DISTINCT created_at) FROM transactions GROUP BY hash HAVING
COUNT(DISTINCT created_at) > 1` is run nightly as part of data integrity checks.
  Any row surfaced indicates parser bug, not legitimate state.

### `memo` storage (fix #4)

**Decision:**

- `transactions.memo` column type changes from `VARCHAR(128)` to **`BYTEA`**.
  Nullable (NULL when `memo_type = 'none'`).
- `transactions.memo_type` retains `VARCHAR(8)` with a `CHECK` constraint
  (see fix #5 below).
- API-layer encoding rules (normative for response serialization):
  - `memo_type = 'text'`: attempt UTF-8 decode. If successful, return as JSON
    string. If decode fails, return as `{"base64": "..."}` object.
  - `memo_type = 'id'`: 8 bytes big-endian → u64 → decimal string.
  - `memo_type = 'hash'`, `'return'`: 32 bytes → lowercase hex.
  - `memo_type = 'none'`: memo is `NULL` / omitted.
- No index on `memo`. Not filtered on at the API level.

### CHECK constraints (fix #5)

**Decision — add `CHECK` to exactly these columns** (all backed by finite,
protocol-fixed enums):

```sql
ALTER TABLE transactions
    ADD CONSTRAINT ck_tx_memo_type
    CHECK (memo_type IN ('none', 'text', 'id', 'hash', 'return'));

ALTER TABLE soroban_events
    ADD CONSTRAINT ck_events_event_type
    CHECK (event_type IN ('contract', 'system', 'diagnostic'));

ALTER TABLE soroban_contracts
    ADD CONSTRAINT ck_contracts_contract_type
    CHECK (contract_type IN ('nft', 'fungible', 'token', 'other'));

ALTER TABLE nft_ownership
    ADD CONSTRAINT ck_nft_event_type
    CHECK (event_type IN ('mint', 'transfer', 'burn'));

ALTER TABLE account_balances_current
    ADD CONSTRAINT ck_abc_asset_type
    CHECK (asset_type IN ('native', 'credit_alphanum4', 'credit_alphanum12',
                          'pool_share', 'contract'));

ALTER TABLE account_balance_history
    ADD CONSTRAINT ck_abh_asset_type
    CHECK (asset_type IN ('native', 'credit_alphanum4', 'credit_alphanum12',
                          'pool_share', 'contract'));
```

**Not added — intentional.** `transaction_participants.role`,
`token_transfers.transfer_type`, `token_transfers.source`,
`operations.type` — each has either evolving membership (new operation types
can appear with protocol upgrades) or is an application-level taxonomy that may
need extension without a migration. Parser-level validation is sufficient.
`tokens.asset_type` CHECK already present from ADR 0012/0013.

### Prefix search (fix #6)

**Decision — add exactly six indexes.** No new tables, no search subsystem.

```sql
-- Prefix exact / LIKE 'XYZ%' on primary lookup keys:
CREATE INDEX idx_tx_hash_prefix
    ON transactions (hash text_pattern_ops);

CREATE INDEX idx_accounts_prefix
    ON accounts (account_id text_pattern_ops);

CREATE INDEX idx_contracts_prefix
    ON soroban_contracts (contract_id text_pattern_ops);

CREATE INDEX idx_pools_prefix
    ON liquidity_pools (pool_id text_pattern_ops);

-- Fuzzy name search (requires pg_trgm):
CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE INDEX idx_nfts_name_trgm
    ON nfts USING GIN (name gin_trgm_ops)
    WHERE name IS NOT NULL;

CREATE INDEX idx_tokens_code_trgm
    ON tokens USING GIN (asset_code gin_trgm_ops)
    WHERE asset_code IS NOT NULL;
```

`tokens.search_vector` and `soroban_contracts.search_vector` (both `TSVECTOR
GENERATED STORED` from ADR 0012/0013) remain unchanged.

### History contract (fix #7)

**Decision — normative contract for state reconstruction:**

| What                                                     | Source            | How                                                          |
| -------------------------------------------------------- | ----------------- | ------------------------------------------------------------ |
| Full ledger replay                                       | S3                | `parsed_ledger_{N}.json` — parser output, write-once         |
| Ledger summary list                                      | DB                | `ledgers` table                                              |
| Transactions in ledger                                   | DB                | `transactions WHERE ledger_sequence = N`                     |
| Transaction detail (normal + advanced)                   | DB + 1× S3        | DB row + `parsed_ledger_{ledger_sequence}.json`              |
| Account current state (summary, balances)                | DB                | `accounts` + `account_balances_current`                      |
| Account transactions (any role)                          | DB                | `transaction_participants` + `transactions` join             |
| Account balance at ledger N                              | DB                | `account_balance_history` (partition pruned on `created_at`) |
| NFT current owner                                        | DB                | `nfts.current_owner` (denormalized, watermark-guarded)       |
| NFT ownership at ledger N                                | DB                | `nft_ownership`                                              |
| NFT transfer history                                     | DB                | `nft_ownership` + `transactions` join                        |
| LP current state                                         | DB                | `liquidity_pool_snapshots` latest-per-pool (LATERAL)         |
| LP state at ledger N                                     | DB                | `liquidity_pool_snapshots`                                   |
| LP chart (time-series)                                   | DB                | `liquidity_pool_snapshots` with partition pruning            |
| LP participants                                          | DB                | `lp_positions`                                               |
| Token transfer list (per token / per account / per pool) | DB                | `token_transfers` indexed lookup                             |
| Contract interface (WASM spec)                           | DB bridge + 1× S3 | `soroban_contracts.wasm_uploaded_at_ledger` → S3             |
| Contract metadata                                        | DB bridge + 1× S3 | `soroban_contracts.deployed_at_ledger` → S3                  |
| Contract invocations list                                | DB                | `soroban_invocations` (slim)                                 |
| Contract invocation detail (args, return)                | DB + 1× S3        | slim row → `parsed_ledger_{ledger_sequence}.json`            |
| Contract events list                                     | DB                | `soroban_events` (slim)                                      |
| Contract event detail (topics, data)                     | DB + 1× S3        | slim row → `parsed_ledger_{ledger_sequence}.json`            |
| Token metadata                                           | DB bridge + 1× S3 | `tokens.metadata_ledger` → S3                                |
| **Not reconstructible from DB (by design)**              | —                 | —                                                            |
| Historical `sequence_number` (nonce)                     | —                 | Intentionally not stored. Only current.                      |
| Historical `home_domain`                                 | —                 | Intentionally not stored. Only current.                      |

**Parser invariants (normative):**

- `transactions.created_at = ledgers.closed_at` for matching `ledger_sequence`.
- `operations.created_at = transactions.created_at` for matching
  `transaction_id` (enforced by composite FK in ADR 0013).
- Same invariant for `soroban_events`, `soroban_invocations`,
  `transaction_participants`, `token_transfers`, `nft_ownership`.
- Parser writes G-form to FK-bearing account columns and optional M-form to
  `*_muxed` siblings. Never M in the FK column.
- `account_balances_current` is upsert with `last_updated_ledger` watermark;
  `account_balance_history` is append-only per `(account_id, ledger_sequence,
asset_type, asset_code, issuer)`.

---

## Detailed schema changes

### Column changes

| Table                      | Column                 | Before                            | After                             | Notes                   |
| -------------------------- | ---------------------- | --------------------------------- | --------------------------------- | ----------------------- |
| `accounts`                 | `account_id`           | `VARCHAR(69)` PK                  | `VARCHAR(56)` PK                  | G-address only          |
| `transactions`             | `source_account`       | `VARCHAR(69) NOT NULL → accounts` | `VARCHAR(56) NOT NULL → accounts` | G-form                  |
| `transactions`             | `source_account_muxed` | —                                 | `VARCHAR(69) NULL`                | NEW; no FK, no index    |
| `transactions`             | `fee_account`          | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          | G-form                  |
| `transactions`             | `fee_account_muxed`    | —                                 | `VARCHAR(69) NULL`                | NEW                     |
| `transactions`             | `memo`                 | `VARCHAR(128)`                    | `BYTEA`                           | binary-safe             |
| `transactions`             | `memo_type`            | `VARCHAR(8)`                      | `VARCHAR(8)` + `CHECK`            | enum                    |
| `operations`               | `source_account`       | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          | G-form                  |
| `operations`               | `source_account_muxed` | —                                 | `VARCHAR(69) NULL`                | NEW                     |
| `operations`               | `destination`          | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          | G-form                  |
| `operations`               | `destination_muxed`    | —                                 | `VARCHAR(69) NULL`                | NEW                     |
| `operations`               | `asset_issuer`         | `VARCHAR(56) → accounts`          | unchanged                         | already 56 per ADR 0013 |
| `transaction_participants` | `account_id`           | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          | G-form                  |
| `soroban_contracts`        | `deployer_account`     | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          |                         |
| `soroban_invocations`      | `caller_account`       | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          |                         |
| `soroban_invocations`      | `caller_account_muxed` | —                                 | `VARCHAR(69) NULL`                | NEW                     |
| `soroban_events`           | `topic0`               | `VARCHAR(32)`                     | `TEXT`                            | no truncation           |
| `soroban_events`           | `event_type`           | `VARCHAR(20)`                     | `VARCHAR(20)` + `CHECK`           | enum                    |
| `soroban_contracts`        | `contract_type`        | `VARCHAR(20)`                     | `VARCHAR(20)` + `CHECK`           | enum                    |
| `token_transfers`          | `from_account`         | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          | G-form                  |
| `token_transfers`          | `from_account_muxed`   | —                                 | `VARCHAR(69) NULL`                | NEW                     |
| `token_transfers`          | `to_account`           | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          | G-form                  |
| `token_transfers`          | `to_account_muxed`     | —                                 | `VARCHAR(69) NULL`                | NEW                     |
| `token_transfers`          | `asset_issuer`         | `VARCHAR(56) → accounts`          | unchanged                         |                         |
| `nfts`                     | `current_owner`        | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          | G-form                  |
| `nft_ownership`            | `owner_account`        | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          | G-form; NULL on burn    |
| `nft_ownership`            | `event_type`           | `VARCHAR(20)`                     | `VARCHAR(20)` + `CHECK`           | enum                    |
| `liquidity_pools`          | `asset_a_issuer`       | `VARCHAR(56) → accounts`          | unchanged                         |                         |
| `liquidity_pools`          | `asset_b_issuer`       | `VARCHAR(56) → accounts`          | unchanged                         |                         |
| `lp_positions`             | `account_id`           | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          | G-form                  |
| `account_balances_current` | `account_id`           | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          |                         |
| `account_balances_current` | `asset_type`           | `VARCHAR(20)`                     | `VARCHAR(20)` + `CHECK`           | enum                    |
| `account_balance_history`  | `account_id`           | `VARCHAR(69) → accounts`          | `VARCHAR(56) → accounts`          |                         |
| `account_balance_history`  | `asset_type`           | `VARCHAR(20)`                     | `VARCHAR(20)` + `CHECK`           | enum                    |

### Indexes added

```sql
CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE INDEX idx_tx_hash_prefix       ON transactions (hash text_pattern_ops);
CREATE INDEX idx_accounts_prefix      ON accounts (account_id text_pattern_ops);
CREATE INDEX idx_contracts_prefix     ON soroban_contracts (contract_id text_pattern_ops);
CREATE INDEX idx_pools_prefix         ON liquidity_pools (pool_id text_pattern_ops);
CREATE INDEX idx_nfts_name_trgm       ON nfts USING GIN (name gin_trgm_ops) WHERE name IS NOT NULL;
CREATE INDEX idx_tokens_code_trgm     ON tokens USING GIN (asset_code gin_trgm_ops) WHERE asset_code IS NOT NULL;
```

### Constraints added

All listed in fix #5 above. Six `CHECK` constraints on enum columns.

### FK graph

**Unchanged from ADR 0013**, with one effect from the muxed narrowing: every
FK column referencing `accounts(account_id)` now has the same `VARCHAR(56)`
type as the parent PK. Previously some were `VARCHAR(69)` referencing
`VARCHAR(69)` — functionally equivalent, but narrower now. No FK is added or
removed. `ledgers` remains a non-hub; no table has an FK to
`ledgers(sequence)`.

### Tables unchanged structurally

`ledgers`, `wasm_interface_metadata`, `tokens`, `liquidity_pools`,
`liquidity_pool_snapshots`, `lp_positions` — no column type changes beyond
those listed. `wasm_interface_metadata` remains in scope pending task 0118
resolution (ADR 0013 deferral stands).

### Tables added

**None.** All seven issues are resolved in place.

---

## Rationale

### Why G-address canonical (not M)

SEP-0023 defines muxed M-addresses as payment-routing hints: a muxed account is
an (ed25519 public key, u64 sub-id) pair. The ed25519 public key **is** the
account on-chain — it owns balances, signs transactions, consumes sequence
numbers. The u64 sub-id is a hint for off-chain systems (exchanges, hosted
wallets) to disambiguate customers sharing one underlying account. Treating
M-form as account identity in a block explorer fragments real account state
across synthetic sub-identities that the chain itself does not recognize as
distinct accounts. Every mature Stellar explorer (stellarchain.io,
stellar.expert) aggregates on G. Our `accounts` table must follow.

Keeping M-form in `*_muxed` columns preserves traceability for advanced/detail
views (a user viewing the raw transaction can still see the original M value)
without letting it leak into the relational identity layer.

### Why `topic0 TEXT`, not a larger VARCHAR

Postgres `VARCHAR(N)` and `TEXT` share the same on-disk storage. The only
difference is the length check. Any `VARCHAR(N)` we picked would be arbitrary —
256? 1024? — without a protocol-defined upper bound. `TEXT` is the honest
type: topics are bounded by ScVal encoding rules and the B-tree limit, not by
an arbitrary schema constant. Parser's canonicalization rules (section Decision)
are the real contract on what goes in.

### Why hash uniqueness relies on parser invariant (not a hash-index table)

Adding an unpartitioned `transaction_hash_index (hash PRIMARY KEY, ...)` table
would add a write-path entry per transaction (~300M rows on mainnet) for the
sole purpose of defending against our own parser replaying the same ledger
with inconsistent `created_at`. That is a bug category the invariant already
rules out by construction — the parser reads `ledgers.closed_at` from the
`parsed_ledger_{N}.json` and writes it into every child row. A monitoring
query catches any regression. Adding the table has real cost (storage, write
path, index maintenance) for a defense-in-depth benefit we can achieve with
one invariant and a nightly check.

The principle "lightweight bridge DB" beats "belt-and-braces integrity" here,
explicitly.

### Why `memo BYTEA`, not two columns

Stellar memo is one of five types. `memo_text` is 28 binary bytes (may be
non-UTF-8 — e.g., hosted-wallet memos often use raw bytes). `memo_hash` /
`memo_return` are 32 raw bytes. `memo_id` is u64. `memo_none` is nothing.
Storing all legal values in `BYTEA` costs no more than VARCHAR and
round-trips any legal memo byte-for-byte. The `memo_type` discriminator tells
the API layer how to decode. Separating `memo_text BYTEA` + `memo_hash BYTEA`

- `memo_id BIGINT` adds four nullable columns for no semantic gain.

### Why CHECK only on six columns

CHECK constraints pay for themselves when the enum is fixed by protocol and
unlikely to grow without a migration anyway. The six chosen (`memo_type`,
`event_type` in events, `contract_type`, `nft_ownership.event_type`,
`asset_type` on both balance tables) meet that test. `operations.type` does
not — new operation types ship with protocol upgrades (BUMP_FOOTPRINT_TTL in
protocol 22 was recent). `transfer_type` and `role` are application-level
groupings that can extend without upstream protocol changes. Putting CHECK on
them increases migration churn for zero integrity gain.

### Why only six search indexes

`/search` behavior decomposes into three access patterns:

1. **Exact match** on primary key → already served by existing PK indexes.
2. **Prefix match** on primary key (user types `GABC...`) → needs
   `text_pattern_ops`.
3. **Fuzzy name** on human-readable strings → trigram GIN on the two columns
   (NFT name, token code) where name-based discovery matters.

Everything else in the endpoint contract is an exact-match redirect on a key
we already have. No `search_index` generic table, no materialized view, no
additional tsvector beyond what ADR 0013 already generates on
`soroban_contracts.name` and `tokens`.

### Why no history beyond what's here

The project requirement is "reconstructable history of ledger-linked state".
All listed state — balance changes, NFT ownership, LP reserves, invocation
timelines, event timelines, transfer timelines — is reconstructable from the
tables defined in ADR 0012/0013 with the contract spelled out in section
Decision. Sequence number and home_domain history are explicitly excluded by
the brief. There is no gap. Adding a generic audit log or event store would
violate "no scope creep without hard justification".

---

## Consequences

### Stellar/XDR compliance

- **Positive:** canonical G-identity aligns with SEP-0023 semantics. `topic0`
  no longer truncates legal CAP-67 topics. `memo` round-trips arbitrary legal
  bytes. CHECK constraints match XDR enum membership.
- **Negative:** parser must implement M→G resolution (trivial per SEP-0023:
  M-address encoding carries G in the body) and must populate two columns
  where previously one sufficed. Slight increase in parse-phase work per
  participant-role row.

### Database weight

- **Net change:** ~neutral. Column type changes (69→56 on account columns)
  save a few bytes per row; adding 7 `*_muxed VARCHAR(69) NULL` columns adds a
  few bytes per row where populated (sparse — most accounts are not muxed).
  Six index additions at mainnet scale total ~a few GB: `text_pattern_ops` on
  four primary-key columns is marginal because those indexes already exist as
  B-tree PKs — `text_pattern_ops` is an additional index variant, but on
  already-small tables (`accounts` ~200 MB, `soroban_contracts` <1 GB,
  `liquidity_pools` <100 MB). `idx_tx_hash_prefix` is the one that grows with
  transaction volume (~60-100 GB ceiling, but only an alternate index on an
  already-small column). Trigram GIN indexes on NFT names and token codes are
  sub-GB.
- **No new tables**, so the "lightweight bridge" property is preserved as
  defined in ADR 0012/0013.

### Ledger-linked history

- **Positive:** parser invariants are now normative, not implicit. Monitoring
  query on `(hash, created_at)` uniqueness catches invariant drift.
- **Positive:** muxed normalization means account history is unified per
  ed25519 public key — `/accounts/:id` and `/accounts/:id/transactions` become
  semantically correct where they were fragmented.
- **No regression** on any existing history path from ADR 0013.

### Endpoint performance

- **Positive:** `/search` with prefix input becomes index-backed instead of
  sequential scan.
- **Positive:** CHECK constraints eliminate a class of runtime bugs at write
  time rather than at query time.
- **Neutral:** `topic0 TEXT` does not measurably change index performance
  versus `VARCHAR(32)` for values that fit in 32 bytes; for values that did
  not, it converts "wrong result" into "correct result".
- **Neutral:** `memo BYTEA` uses same storage as `VARCHAR(128)`; API-layer
  decoding is negligible.

### Ingest simplicity

- **Slight increase:** parser now computes M→G for any muxed address, writes
  two columns instead of one, and asserts `created_at = ledgers.closed_at`
  explicitly. All three are trivial.
- **No increase** in transaction count, batch size, or commit frequency.

### Replay / re-ingest risk

- **Reduced.** Re-ingest protocol is normative: `DELETE WHERE ledger_sequence
= N` → cascade → re-INSERT. Combined with the `created_at = closed_at`
  invariant, double-ingest of the same ledger is idempotent. Nightly
  monitoring catches any violation.

### Operational cost

- **No change** in S3 file structure or size.
- **No change** in RDS instance class assumptions.
- One extension (`pg_trgm`) — standard Postgres contrib, no operational cost
  beyond enabling.

---

## Migration / rollout notes

Applies only to environments where ADR 0013 is already implemented. For
greenfield deployment, incorporate directly into the initial DDL.

Migration is **not required to be online** — the system is pre-GA. A brief
maintenance window is acceptable. Order:

1. **Extension.** `CREATE EXTENSION IF NOT EXISTS pg_trgm;`
2. **Add `*_muxed` columns** on all seven sites (nullable, no default, no
   index). `ALTER TABLE ... ADD COLUMN` is metadata-only in Postgres for
   nullable columns without defaults — instantaneous.
3. **Backfill muxed columns** (if rows already exist with M-form in the FK
   column): one-time script that parses existing `source_account` etc. values,
   extracts G into the FK column and M into `*_muxed`. Runs per-partition;
   acceptable under the maintenance window.
4. **Narrow account columns** `VARCHAR(69) → VARCHAR(56)` via
   `ALTER COLUMN TYPE`. Postgres does a full table rewrite; acceptable under
   maintenance. All existing values must already fit in 56 chars after step
   3 — if any don't, that's a parser bug to fix first.
5. **Change `memo` type** `VARCHAR(128) → BYTEA` via `ALTER COLUMN TYPE memo
TYPE BYTEA USING convert_to(memo, 'UTF8')` (lossy for any non-UTF-8 memos
   already written; in a fresh system, none exist).
6. **Change `topic0`** `VARCHAR(32) → TEXT` via `ALTER COLUMN TYPE`.
   Zero-copy — Postgres does not rewrite for TEXT widening.
7. **Add CHECK constraints** — `ALTER TABLE ... ADD CONSTRAINT ... CHECK (...)
NOT VALID` followed by `ALTER TABLE ... VALIDATE CONSTRAINT ...`. Split
   reduces lock time; acceptable online even outside the window.
8. **Create the six new indexes.** `CREATE INDEX CONCURRENTLY` on the four
   non-partitioned tables; for any index on a partitioned table, create on
   each partition concurrently then attach. None of the six target partitioned
   tables except `idx_tx_hash_prefix` — create per-partition.
9. **Parser update** to:
   - write G in FK columns and M in `*_muxed`;
   - canonicalize `topic0` per rules;
   - assert `transactions.created_at = ledgers.closed_at` invariant;
   - write `memo` as `BYTEA`.
10. **Enable nightly monitoring query**:
    ```sql
    SELECT hash, COUNT(DISTINCT created_at)
    FROM transactions
    GROUP BY hash
    HAVING COUNT(DISTINCT created_at) > 1;
    ```
    Alert if result is non-empty.

Rollback: the schema changes are all reversible (`ALTER COLUMN TYPE` in
reverse, `DROP CONSTRAINT`, `DROP INDEX`). Parser changes revert with the
code rollback.

---

## Open questions

None that block this ADR. Two deliberately deferred topics (out of scope here,
tracked elsewhere):

- **Soroban AMM pools** (Soroswap, Phoenix). Whether these appear in
  `/liquidity-pools` is an API-surface decision pending product confirmation,
  not a schema decision. If they must appear, a follow-up ADR adds
  `pool_type` and `contract_id` to `liquidity_pools`. If not, no change.
- **Removal of `wasm_interface_metadata` staging table** in favor of direct
  population of `soroban_contracts`. Contingent on task 0118's in-memory
  parser cache landing, per ADR 0013 deferral. Follow-up ADR when 0118 is
  merged.

---

## References

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0012: Lightweight bridge DB schema revision](0012_lightweight-bridge-db-schema-revision.md)
- [ADR 0013: Sequential ingest schema with full FK integrity](0013_sequential-ingest-full-fk-schema.md)
- [SEP-0023: Muxed Accounts](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0023.md)
- [CAP-0067: Unified Events](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0067.md)
- [Stellar XDR: Transaction and Memo definitions](https://github.com/stellar/stellar-xdr)
- [PostgreSQL: Operator Classes (`text_pattern_ops`)](https://www.postgresql.org/docs/current/indexes-opclass.html)
- [PostgreSQL: `pg_trgm` extension](https://www.postgresql.org/docs/current/pgtrgm.html)
