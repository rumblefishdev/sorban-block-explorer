---
id: '0017'
title: 'Ingest guard clarification, topic0 validation, final schema'
status: proposed
deciders: [fmazur]
related_tasks: []
related_adrs: ['0011', '0012', '0013', '0014', '0015', '0016']
tags: [database, schema, stellar, ingest, soroban-events, final-state]
links: []
history:
  - date: 2026-04-19
    status: proposed
    who: fmazur
    note: 'ADR created — closes out three residual ambiguities from ADR 0016 and documents the final schema state'
---

# ADR 0017: Ingest guard clarification, `topic0` validation, final schema

**Related:**

- [ADR 0013: Sequential ingest schema with full FK integrity](0013_sequential-ingest-full-fk-schema.md)
- [ADR 0014: Schema fixes — Stellar/XDR compliance](0014_schema-fixes-stellar-xdr-compliance.md)
- [ADR 0015: Hash index, typed topic0, migration honesty, CHECK policy](0015_hash-index-topic-typing-migration-honesty.md)
- [ADR 0016: Hash fail-fast, topic0 pre-GA unification, filter contract](0016_hash-fail-fast-topic-unification-filter-contract.md)

---

## Status

`proposed` — corrective delta on top of ADR 0016. Overrides ADR 0016 on
three specific points listed below; adds one DDL-level constraint. Every
other decision in ADR 0011–0016 stands.

This ADR also serves as the **final, consolidated schema reference** —
the last section reproduces the complete model after all deltas from
ADR 0012 through ADR 0017.

---

## Context

Review of ADR 0016 surfaced three residual ambiguities that the present
ADR closes:

1. **Role of the pre-`BEGIN` `SELECT EXISTS FROM ledgers` check is described
   too strongly.** ADR 0016 introduces the check as part of the
   "idempotency contract" and places it alongside the PK constraints in
   the prose. This reads as if the check itself provides integrity.
   It does not — two workers can both observe "ledger not present" and
   both proceed. Integrity comes from the `ledgers.sequence` PK and from
   the atomicity of the ledger-group commit, not from the pre-flight
   SELECT. The wording needs to separate "retry fast-skip" from "final
   integrity barrier" so that readers do not misattribute the guarantee.

2. **Post-migration `topic0` validation is heuristic.** ADR 0016 suggests
   `WHERE topic0 NOT LIKE '%:%'` as the completeness check for the
   one-time re-ingest. This produces false negatives for any pre-typed
   string that happened to contain a colon (any base64 string that
   included `:` before padding normalization, any pre-canonicalized
   URL-like string). The validation must be precise — a whitelist of
   legal `type_code` prefixes, not a substring heuristic.

3. **`idx_events_topic0`'s design intent is implicit.** ADR 0015 and ADR
   0016 speak of equality filtering on `topic0` without spelling out
   that the supporting index is `(contract_id, topic0, created_at DESC)`
   — a **scoped-by-contract** index. The endpoint that uses it is
   `GET /contracts/:contract_id/events`, which always carries the
   `contract_id` prefix. Any use that attempts `WHERE topic0 = ?` without
   `contract_id` would not be index-backed on this column order. That
   limitation is deliberate — global topic0 search is not an endpoint —
   but it has never been written down. Without it, a future reader may
   assume capabilities the schema does not provide.

Fixing these three closes the corrective-ADR sequence (0014 → 0015 → 0016
→ 0017). This ADR does not open new territory; it tightens the contracts
and then freezes the model state.

---

## Decision

### Summary of decisions

- **The pre-`BEGIN` `SELECT EXISTS FROM ledgers` check is a retry
  optimization, not an integrity barrier.** Final integrity is provided
  by the `ledgers.sequence` PRIMARY KEY, the `transaction_hash_index.hash`
  PRIMARY KEY, the ledger-group transaction atomicity, and the fail-fast
  inserts introduced in ADR 0016. Even if the SELECT were removed
  entirely, duplicate ledgers could not land. The SELECT exists only to
  skip work, not to guarantee correctness.
- **Post-migration `topic0` validation uses a precise whitelist** of the
  18 legal `type_code` prefixes defined in ADR 0015's ScVal encoding
  table. The whitelist is anchored at the start of the string (`^`) and
  requires the `:` separator, eliminating coincidental matches. The same
  whitelist is promoted to a **database-level `CHECK` constraint** so
  that going forward, no row in non-typed form can ever enter
  `soroban_events.topic0` — closing the gap between migration validity
  and ongoing ingest correctness.
- **`idx_events_topic0 (contract_id, topic0, created_at DESC)` is
  explicitly documented as a scoped-by-contract index.** The supported
  access pattern is `WHERE contract_id = ? [AND topic0 = ?] ORDER BY
created_at DESC`. Global search on `topic0` without `contract_id` is
  not part of the contract, not index-backed, and not added as an
  endpoint. This is a deliberate scope boundary.

### Fix 1: ingest guard clarification

**Decision — the integrity layers, in order:**

1. **Retry fast-skip (optimization, not integrity):**
   `SELECT EXISTS (SELECT 1 FROM ledgers WHERE sequence = $N)` issued
   before `BEGIN`. If the row exists, the worker skips the ledger. This
   saves wasted work on a ledger that has already been committed.
2. **Transaction atomicity (integrity):**
   The ledger-group commit is one PostgreSQL transaction. All inserts
   succeed together or all roll back together.
3. **`ledgers.sequence` PRIMARY KEY (integrity barrier for ledger
   uniqueness):**
   Any attempt to insert a second `ledgers` row for the same sequence
   (due to a race between two workers, a missed fast-skip, or a parser
   bug) fails with `unique_violation`. The containing transaction rolls
   back entirely. At most one `ledgers` row per sequence can ever exist.
4. **`transaction_hash_index.hash` PRIMARY KEY (integrity barrier for
   hash global uniqueness):**
   Fail-fast on any duplicate hash. Same rollback semantics.
5. **Composite FKs to `transactions(id, created_at)` (integrity barrier
   for child–parent coupling):**
   Already established in ADR 0013. Ensure every child row's parent tx
   exists in the same ledger partition.

**The normative language** for operator and developer documentation is:

> The pre-BEGIN `SELECT EXISTS` on `ledgers` is a retry optimization that
> avoids re-processing completed ledgers. It is not the source of
> integrity. Integrity for ledger uniqueness and hash uniqueness is
> enforced by the `ledgers.sequence` and `transaction_hash_index.hash`
> primary keys, combined with the atomicity of the ledger-group
> transaction. Removing the pre-BEGIN check would degrade performance
> on retry paths but would not permit duplicate data.

### Fix 2: precise `topic0` validation + going-forward CHECK

**Decision:**

- **The exhaustive set of legal `type_code` prefixes** for `topic0` is:

  ```
  sym str bool void u32 i32 u64 i64 u128 i128 u256 i256
  tp dur bytes addr err xdr
  ```

  (18 codes, mapping 1:1 to the ScVal encoding table in ADR 0015. `xdr:`
  is the fallback for composite ScVals.)

- **Post-migration completeness query** (replaces ADR 0016's
  `NOT LIKE '%:%'`):

  ```sql
  SELECT COUNT(*) FROM soroban_events
  WHERE topic0 IS NOT NULL
    AND topic0 !~ '^(sym|str|bool|void|u32|i32|u64|i64|u128|i128|u256|i256|tp|dur|bytes|addr|err|xdr):';
  ```

  Expected result: `0`. Any non-zero result means the migration did not
  fully cover the table and the operator must identify the remaining
  range and re-ingest.

- **Database-level CHECK constraint (NEW in this ADR)**:

  ```sql
  ALTER TABLE soroban_events
      ADD CONSTRAINT ck_events_topic0_typed
      CHECK (
          topic0 IS NULL OR
          topic0 ~ '^(sym|str|bool|void|u32|i32|u64|i64|u128|i128|u256|i256|tp|dur|bytes|addr|err|xdr):'
      );
  ```

  Rationale under ADR 0015's CHECK policy: this is a **category A
  constraint** (protocol-fixed enum). The set of ScVal variants is
  defined by Soroban protocol (CAP-67, soroban-env). Additions to the
  variant set would ship with a protocol upgrade, at which point the
  parser's encoding rules and this CHECK constraint migrate together —
  the expected coupling. CHECK cost is metadata-only; storage and write
  path are unchanged. Going forward, no row may enter
  `soroban_events.topic0` in any other form.

- **Constraint is added with `NOT VALID` first, then `VALIDATE` after
  re-ingest completes.** This lets the constraint be created without
  blocking on a full-table scan mid-migration; the validation step runs
  after the migration has produced a clean state.

### Fix 3: `idx_events_topic0` scope is explicit

**Decision:**

- The supported access pattern for `soroban_events` queries involving
  `topic0` is:

  ```
  WHERE contract_id = ?
    [ AND topic0 = ? ]
    [ AND created_at BETWEEN ? AND ? ]
  ORDER BY created_at DESC
  ```

  Backed by `idx_events_topic0 (contract_id, topic0, created_at DESC)
WHERE topic0 IS NOT NULL`.

- **Not supported (explicit non-capability):**

  - `WHERE topic0 = ?` without `contract_id` — no index-backed path.
  - `WHERE topic0 LIKE ?` — ruled out by ADR 0016's equality-only
    contract.
  - Any cross-contract `topic0` aggregation on the fly.

- **Endpoint alignment:** the only API endpoint that filters on `topic0`
  is `GET /contracts/:contract_id/events`, which always provides the
  `contract_id` prefix. The index and the endpoint are co-designed. No
  other endpoint consumes `topic0` directly.

- **No DDL change.** The index already exists in its correct form from
  ADR 0012/0013. This fix is a documentation tightening, not a schema
  change.

---

## Detailed schema changes

### DDL changes

Exactly one DDL change in this ADR:

```sql
ALTER TABLE soroban_events
    ADD CONSTRAINT ck_events_topic0_typed
    CHECK (
        topic0 IS NULL OR
        topic0 ~ '^(sym|str|bool|void|u32|i32|u64|i64|u128|i128|u256|i256|tp|dur|bytes|addr|err|xdr):'
    )
    NOT VALID;
-- After topic0 re-ingest completes:
ALTER TABLE soroban_events VALIDATE CONSTRAINT ck_events_topic0_typed;
```

### Non-DDL contracts changed

| Contract                               | Before (ADR 0016)                      | After (this ADR)                               | Artifact                |
| -------------------------------------- | -------------------------------------- | ---------------------------------------------- | ----------------------- | ------ | ------------------- |
| Role of `SELECT EXISTS FROM ledgers`   | ambiguous — described as "idempotency" | explicit retry optimization; integrity via PKs | runbook, developer docs |
| Post-migration `topic0` validation     | `NOT LIKE '%:%'` heuristic             | regex whitelist `^(sym                         | str                     | ...):` | migration checklist |
| `topic0` filter contract documentation | equality-only (ADR 0016)               | equality-only + always scoped by `contract_id` | API docs                |

### Nothing else changes

No new tables. No new indexes. No new columns. No column type changes.
No changes to partitioning, FK graph, or CHECK constraints beyond the
single one above.

---

## Rationale

### Why `SELECT EXISTS` is optimization, not integrity

An integrity barrier is one that, if removed, allows incorrect data to
persist. Remove the fast-skip: a retried worker executes the full
ledger-group transaction, the first INSERT hits `ledgers.sequence` PK,
fails with `unique_violation`, transaction rolls back. No duplicate data
exists. The retry is slower — it did the work of parsing the ledger,
computing IDs, preparing rows — but correctness is preserved by the PK,
not by the SELECT.

That is the correct way to describe the layered model. Describing the
SELECT as part of the integrity guarantee misleads. If someone later
thinks "we can skip the SELECT because we have the PK" and removes it,
correctness holds. If someone thinks "we can relax the PK because we
have the SELECT" and removes the PK, correctness breaks. The honest
description must make the second misstep impossible.

### Why regex whitelist + CHECK, not just a better migration query

ADR 0016's `NOT LIKE '%:%'` is a one-shot heuristic. It verifies nothing
beyond "contains a colon." The typed form has strict structure — 18
legal prefixes, each followed by `:`. Validating against that structure
requires a regex anchored at `^`. Once we have the regex, promoting it
to a CHECK constraint costs nothing and provides going-forward fail-fast
against any future parser regression that could reintroduce old-format
values. The two uses (post-migration validation + going-forward
enforcement) share one source of truth.

The constraint fits ADR 0015's CHECK policy cleanly (category A:
protocol-fixed enum of ScVal variants). A new Soroban protocol adding a
ScVal variant would require a parser update anyway; the CHECK
constraint migration is done in the same change, same commit.

### Why `idx_events_topic0` deserves explicit scope documentation

The index column order `(contract_id, topic0, created_at DESC)` is
optimal for the scoped endpoint but suboptimal for any hypothetical
global `topic0` search. Without explicit documentation, a future
developer might propose "let's add an endpoint that searches all events
with topic0 = X" and assume the index supports it. It doesn't — the
prefix column is `contract_id`, and without it the planner would not
use the index efficiently. Making this explicit saves one avoidable
misstep.

The decision to not support global `topic0` search is principled:

- No documented endpoint needs it.
- Adding a global index (`(topic0, created_at DESC)`) is extra storage
  (~10-30 GB at mainnet scale on the events table) for a capability
  nobody has requested.
- The "no projecting for hypothetical future needs" principle rules it
  out.

Documenting the scope matches what we already built; adding a new index
would violate minimalism.

### What this ADR refuses to do

- No additional index on `soroban_events.topic0` without `contract_id`.
- No CHECK constraint on `topic0` type codes beyond the 18 already
  defined in ADR 0015.
- No refactor of the pre-BEGIN check into a "proper" idempotency
  mechanism (distributed lock, advisory lock, etc.) — the PK already
  provides what the system needs.

---

## Consequences

### Stellar/XDR compliance

- **Positive.** The `topic0` CHECK constraint enforces at write time
  that stored values correspond to the ScVal variant set defined by
  Soroban protocol. A parser regression that bypassed typed encoding
  would be caught immediately.
- **Neutral.** Other XDR compliance properties from ADR 0014 are
  unchanged.

### Database weight

- **No change.** One CHECK constraint is metadata-only. No new rows, no
  new indexes, no new columns.
- Principle "lightweight bridge DB" preserved exactly as in ADR 0016.

### History correctness

- **Improved.** Post-migration validation now catches any residual
  old-format rows with certainty, not heuristic. Going forward, the CHECK
  makes the typed-form invariant a schema-level property, not just a
  parser contract.

### Endpoint performance

- **Unchanged.** CHECK constraints are not consulted on SELECT, only on
  INSERT/UPDATE. All read paths remain identical.

### Ingest simplicity

- **Unchanged.** Parser was already writing typed form after ADR 0015;
  the CHECK only surfaces errors that would have been silent data bugs.

### Replay / re-ingest risk

- **Reduced.** CHECK constraint would cause a re-ingest with a buggy
  parser to fail fast on bad rows, surfacing the bug immediately rather
  than landing bad data in the table.

### Operational cost

- **Marginal.** One `ALTER TABLE ADD CONSTRAINT ... NOT VALID` plus one
  `VALIDATE CONSTRAINT` after re-ingest. Runbook gains one validation
  query (regex-based) replacing the prior heuristic.

### Consistency of historical data contract

- **Improved.** Post-migration invariant on `topic0` becomes a DB-level
  property. Historical query results are now guaranteed era-independent
  in format (guaranteed by ADR 0016's unification) and in type fidelity
  (guaranteed by this ADR's CHECK).

### Still a lightweight bridge?

Yes. The entire ADR 0017 delta is:

- one CHECK constraint (metadata only),
- three documentation tightenings (no artifacts).

No growth in stored data, indexes, or write amplification. The model
from ADR 0011's opening principle — DB as lightweight index, S3 as full
parsed payload — is preserved unchanged.

---

## Migration / rollout notes

Applies to environments where ADR 0016 is in effect. Greenfield
deployments pick up the CHECK constraint in the initial DDL and skip the
migration-validation step.

1. **Apply the CHECK constraint in `NOT VALID` mode** before completing
   the `topic0` re-ingest:

   ```sql
   ALTER TABLE soroban_events
       ADD CONSTRAINT ck_events_topic0_typed
       CHECK (
           topic0 IS NULL OR
           topic0 ~ '^(sym|str|bool|void|u32|i32|u64|i64|u128|i128|u256|i256|tp|dur|bytes|addr|err|xdr):'
       )
       NOT VALID;
   ```

   This blocks any new bad-format inserts immediately without requiring
   an in-progress scan.

2. **Run the re-ingest** per ADR 0016's migration notes.

3. **Run the precise validation query:**

   ```sql
   SELECT COUNT(*) FROM soroban_events
   WHERE topic0 IS NOT NULL
     AND topic0 !~ '^(sym|str|bool|void|u32|i32|u64|i64|u128|i128|u256|i256|tp|dur|bytes|addr|err|xdr):';
   ```

   Expected: `0`. If non-zero, re-ingest the affected partitions and
   re-run until zero.

4. **Validate the constraint** (moves it from `NOT VALID` to fully
   enforced including historical rows):

   ```sql
   ALTER TABLE soroban_events VALIDATE CONSTRAINT ck_events_topic0_typed;
   ```

5. **Update developer and operator documentation:**
   - Runbook: the pre-BEGIN `SELECT EXISTS` is labeled "retry
     optimization"; the integrity model is documented as "PK + atomic
     transaction."
   - API docs: `filter[topic0]` is documented as equality-only, always
     scoped by `contract_id` via the endpoint path.

Rollback: `DROP CONSTRAINT ck_events_topic0_typed`. The constraint does
not alter data; dropping it is a metadata operation.

---

## Final schema (complete)

This section reproduces the complete model state after all deltas from
ADR 0011 through ADR 0017. It is the authoritative schema reference.
Every table, column, constraint, and index listed here is part of the
production model.

**Conventions:**

- Account addresses (G-form): `VARCHAR(56)`.
- Optional muxed siblings (SEP-0023 routing hints): `VARCHAR(69) NULL`,
  never FK-referenced, never indexed.
- Contract IDs and pool IDs: `VARCHAR(56)` / `VARCHAR(64)`.
- Raw token amounts (i128, SEP-0041): `NUMERIC(39,0)`.
- Computed amounts (TVL, volume, shares): bare `NUMERIC`.
- Hashes (SHA-256 hex): `VARCHAR(64)`.
- Binary fields (memo): `BYTEA`.
- Partitioning: monthly by `created_at` on high-volume time-series
  tables.
- **No FK references** `ledgers(sequence)`. `ledger_sequence` is a
  bridge column to S3 (`parsed_ledger_{N}.json`) and an index target,
  not a relational hub.
- `pg_trgm` extension enabled.

---

### 1. `ledgers`

Role: timeline for `/ledgers` endpoints. Not a relational hub. Bridge
to S3 via `sequence`.

```sql
CREATE TABLE ledgers (
    sequence          BIGINT PRIMARY KEY,
    hash              VARCHAR(64) NOT NULL UNIQUE,
    closed_at         TIMESTAMPTZ NOT NULL,
    protocol_version  INTEGER NOT NULL,
    transaction_count INTEGER NOT NULL,
    base_fee          BIGINT NOT NULL
);
CREATE INDEX idx_ledgers_closed_at ON ledgers (closed_at DESC);
```

`ledgers.sequence` PK is the **integrity barrier for ledger uniqueness**
(ADR 0017 fix #1).

---

### 2. `accounts`

Role: current-state account registry, keyed by G-address. History of
`sequence_number` and `home_domain` is intentionally not kept.

```sql
CREATE TABLE accounts (
    account_id        VARCHAR(56) PRIMARY KEY,          -- G-form only
    first_seen_ledger BIGINT NOT NULL,
    last_seen_ledger  BIGINT NOT NULL,
    sequence_number   BIGINT NOT NULL,
    home_domain       VARCHAR(256)
);
CREATE INDEX idx_accounts_last_seen ON accounts (last_seen_ledger DESC);
CREATE INDEX idx_accounts_prefix    ON accounts (account_id text_pattern_ops);
```

---

### 3. `transactions` (partitioned monthly by `created_at`)

Role: per-transaction index row. Heavy payloads (`envelope_xdr`,
`result_xdr`, `result_meta_xdr`, `operation_tree`) live on S3.

```sql
CREATE TABLE transactions (
    id                    BIGSERIAL,
    hash                  VARCHAR(64) NOT NULL,
    ledger_sequence       BIGINT NOT NULL,                -- bridge to S3
    application_order     SMALLINT NOT NULL,
    source_account        VARCHAR(56) NOT NULL REFERENCES accounts(account_id),
    source_account_muxed  VARCHAR(69),                    -- SEP-0023 routing hint, no FK
    fee_charged           BIGINT NOT NULL,
    fee_account           VARCHAR(56) REFERENCES accounts(account_id),
    fee_account_muxed     VARCHAR(69),                    -- no FK
    is_fee_bump           BOOLEAN NOT NULL DEFAULT FALSE,
    inner_tx_hash         VARCHAR(64),
    successful            BOOLEAN NOT NULL,
    result_code           VARCHAR(30),
    operation_count       SMALLINT NOT NULL,
    has_soroban           BOOLEAN NOT NULL DEFAULT FALSE,
    memo_type             VARCHAR(8),
    memo                  BYTEA,                          -- binary-safe
    parse_error           BOOLEAN NOT NULL DEFAULT FALSE,
    parse_error_reason    TEXT,
    created_at            TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),
    UNIQUE (hash, created_at),
    CONSTRAINT ck_tx_memo_type CHECK (
        memo_type IN ('none', 'text', 'id', 'hash', 'return')
    )
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_tx_hash           ON transactions (hash);
CREATE INDEX idx_tx_hash_prefix    ON transactions (hash text_pattern_ops);
CREATE INDEX idx_tx_source_created ON transactions (source_account, created_at DESC);
CREATE INDEX idx_tx_ledger         ON transactions (ledger_sequence, application_order);
CREATE INDEX idx_tx_created        ON transactions (created_at DESC);
CREATE INDEX idx_tx_has_soroban    ON transactions (created_at DESC) WHERE has_soroban;
CREATE INDEX idx_tx_inner_hash     ON transactions (inner_tx_hash) WHERE inner_tx_hash IS NOT NULL;
```

---

### 4. `transaction_hash_index`

Role: globally-unique lookup for `GET /transactions/:hash`, and the
**integrity barrier for hash uniqueness** (ADR 0015 + fail-fast per
ADR 0016).

```sql
CREATE TABLE transaction_hash_index (
    hash            VARCHAR(64) PRIMARY KEY,
    ledger_sequence BIGINT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL
);
-- No FK (lookup table, intentional).
-- No auxiliary indexes (PK is the only access path).
-- Inserts are fail-fast (no ON CONFLICT).
```

---

### 5. `operations` (partitioned monthly by `created_at`)

Role: per-operation index row. Operation `details` JSONB lives on S3.

```sql
CREATE TABLE operations (
    id                    BIGSERIAL,
    transaction_id        BIGINT NOT NULL,
    application_order     SMALLINT NOT NULL,
    source_account        VARCHAR(56) NOT NULL REFERENCES accounts(account_id),
    source_account_muxed  VARCHAR(69),
    type                  VARCHAR(32) NOT NULL,
    destination           VARCHAR(56) REFERENCES accounts(account_id),
    destination_muxed     VARCHAR(69),
    contract_id           VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    function_name         VARCHAR(100),
    asset_code            VARCHAR(12),
    asset_issuer          VARCHAR(56) REFERENCES accounts(account_id),
    pool_id               VARCHAR(64) REFERENCES liquidity_pools(pool_id),
    ledger_sequence       BIGINT NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),
    UNIQUE (transaction_id, application_order, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at)
        ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_ops_tx          ON operations (transaction_id);
CREATE INDEX idx_ops_contract    ON operations (contract_id, created_at DESC)    WHERE contract_id IS NOT NULL;
CREATE INDEX idx_ops_type        ON operations (type, created_at DESC);
CREATE INDEX idx_ops_destination ON operations (destination, created_at DESC)    WHERE destination IS NOT NULL;
CREATE INDEX idx_ops_asset       ON operations (asset_code, asset_issuer, created_at DESC) WHERE asset_code IS NOT NULL;
CREATE INDEX idx_ops_pool        ON operations (pool_id, created_at DESC)        WHERE pool_id IS NOT NULL;
```

No CHECK on `type` — protocol-evolving enum (new op types ship with
protocol upgrades). Parser-level validation only. (ADR 0015 policy
category C.)

---

### 6. `transaction_participants` (partitioned monthly by `created_at`)

Role: N:M link between accounts and transactions across every
participation role. Required for complete
`GET /accounts/:account_id/transactions`.

```sql
CREATE TABLE transaction_participants (
    transaction_id  BIGINT NOT NULL,
    account_id      VARCHAR(56) NOT NULL REFERENCES accounts(account_id),
    role            VARCHAR(16) NOT NULL,
    ledger_sequence BIGINT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (account_id, created_at, transaction_id, role),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at)
        ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_tp_tx ON transaction_participants (transaction_id);
```

No CHECK on `role` — application-level taxonomy, expected to extend
without migration. (ADR 0015 policy category D.)

---

### 7. `soroban_contracts`

Role: Soroban contract registry. Heavy metadata / WASM specs live on
S3, indexed by bridge columns.

```sql
CREATE TABLE soroban_contracts (
    contract_id              VARCHAR(56) PRIMARY KEY,
    wasm_hash                VARCHAR(64),
    wasm_uploaded_at_ledger  BIGINT,                      -- bridge to S3
    deployer_account         VARCHAR(56) REFERENCES accounts(account_id),
    deployed_at_ledger       BIGINT,                      -- bridge to S3
    contract_type            VARCHAR(20) NOT NULL DEFAULT 'other',
    is_sac                   BOOLEAN NOT NULL DEFAULT FALSE,
    name                     VARCHAR(256),
    search_vector            TSVECTOR GENERATED ALWAYS AS (
                                 to_tsvector('simple', coalesce(name, ''))
                             ) STORED,
    CONSTRAINT ck_contracts_contract_type CHECK (
        contract_type IN ('nft', 'fungible', 'token', 'other')
    )
);
CREATE INDEX idx_contracts_type     ON soroban_contracts (contract_type);
CREATE INDEX idx_contracts_wasm     ON soroban_contracts (wasm_hash) WHERE wasm_hash IS NOT NULL;
CREATE INDEX idx_contracts_deployer ON soroban_contracts (deployer_account) WHERE deployer_account IS NOT NULL;
CREATE INDEX idx_contracts_search   ON soroban_contracts USING GIN (search_vector);
CREATE INDEX idx_contracts_prefix   ON soroban_contracts (contract_id text_pattern_ops);
```

---

### 8. `wasm_interface_metadata`

Role: staging for the 2-ledger deploy pattern (WASM uploaded in ledger
N, contract deployed in ledger N+k). Full WASM interface spec lives on
S3.

```sql
CREATE TABLE wasm_interface_metadata (
    wasm_hash           VARCHAR(64) PRIMARY KEY,
    name                VARCHAR(256),
    uploaded_at_ledger  BIGINT NOT NULL,                  -- bridge to S3
    contract_type       VARCHAR(20) NOT NULL DEFAULT 'other'
);
```

May be retired after task 0118's parser-level WASM cache lands. Kept
in current schema per ADR 0013 deferral.

---

### 9. `soroban_events` (partitioned monthly by `created_at`)

Role: per-event index row. Heavy topic/data payloads live on S3.
`topic0` carries typed canonical representation (ADR 0015), validated
by CHECK (this ADR).

```sql
CREATE TABLE soroban_events (
    id               BIGSERIAL,
    transaction_id   BIGINT NOT NULL,
    contract_id      VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    event_type       VARCHAR(20) NOT NULL,
    topic0           TEXT,                                -- typed: "{type_code}:{value}"
    event_index      SMALLINT NOT NULL,
    ledger_sequence  BIGINT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),
    UNIQUE (transaction_id, event_index, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at)
        ON DELETE CASCADE,
    CONSTRAINT ck_events_event_type CHECK (
        event_type IN ('contract', 'system', 'diagnostic')
    ),
    CONSTRAINT ck_events_topic0_typed CHECK (
        topic0 IS NULL OR
        topic0 ~ '^(sym|str|bool|void|u32|i32|u64|i64|u128|i128|u256|i256|tp|dur|bytes|addr|err|xdr):'
    )
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_events_contract ON soroban_events (contract_id, created_at DESC);
CREATE INDEX idx_events_topic0   ON soroban_events (contract_id, topic0, created_at DESC)
    WHERE topic0 IS NOT NULL;
CREATE INDEX idx_events_tx       ON soroban_events (transaction_id);
```

`idx_events_topic0` is **scoped-by-contract** (ADR 0017 fix #3). Use
pattern: `WHERE contract_id = ? [AND topic0 = ?] ORDER BY created_at
DESC`. Global `topic0` search without `contract_id` is **not** an
index-backed access pattern and is not an API endpoint.

---

### 10. `soroban_invocations` (partitioned monthly by `created_at`)

Role: per-invocation index row. Arg / return payloads on S3.

```sql
CREATE TABLE soroban_invocations (
    id                    BIGSERIAL,
    transaction_id        BIGINT NOT NULL,
    contract_id           VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    caller_account        VARCHAR(56) REFERENCES accounts(account_id),
    caller_account_muxed  VARCHAR(69),
    function_name         VARCHAR(100) NOT NULL,
    successful            BOOLEAN NOT NULL,
    invocation_index      SMALLINT NOT NULL,
    ledger_sequence       BIGINT NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),
    UNIQUE (transaction_id, invocation_index, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at)
        ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_inv_contract ON soroban_invocations (contract_id, created_at DESC);
CREATE INDEX idx_inv_function ON soroban_invocations (contract_id, function_name, created_at DESC);
CREATE INDEX idx_inv_caller   ON soroban_invocations (caller_account, created_at DESC)
    WHERE caller_account IS NOT NULL;
CREATE INDEX idx_inv_tx       ON soroban_invocations (transaction_id);
```

---

### 11. `tokens`

Role: token registry (classic, SAC, Soroban native). Heavy metadata on
S3, indexed by `metadata_ledger`.

```sql
CREATE TABLE tokens (
    id                SERIAL PRIMARY KEY,
    asset_type        VARCHAR(20) NOT NULL,
    asset_code        VARCHAR(12),
    issuer_address    VARCHAR(56) REFERENCES accounts(account_id),
    contract_id       VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    name              VARCHAR(256),
    decimals          SMALLINT,                           -- cached SEP-41 value
    metadata_ledger   BIGINT,                             -- bridge to S3
    search_vector     TSVECTOR GENERATED ALWAYS AS (
                          to_tsvector('simple',
                              coalesce(asset_code, '') || ' ' || coalesce(name, ''))
                      ) STORED,
    CONSTRAINT ck_tokens_asset_type CHECK (
        asset_type IN ('native', 'classic', 'sac', 'soroban')
    )
);
CREATE UNIQUE INDEX idx_tokens_classic   ON tokens (asset_code, issuer_address)
    WHERE asset_type IN ('classic', 'sac');
CREATE UNIQUE INDEX idx_tokens_soroban   ON tokens (contract_id)
    WHERE asset_type = 'soroban';
CREATE UNIQUE INDEX idx_tokens_sac       ON tokens (contract_id)
    WHERE asset_type = 'sac';
CREATE INDEX idx_tokens_type   ON tokens (asset_type);
CREATE INDEX idx_tokens_search ON tokens USING GIN (search_vector);
CREATE INDEX idx_tokens_code_trgm ON tokens USING GIN (asset_code gin_trgm_ops)
    WHERE asset_code IS NOT NULL;
```

---

### 12. `token_transfers` (partitioned monthly by `created_at`)

Role: canonical transfer ledger unifying classic payments and SEP-41
events. Required for `GET /tokens/:id/transactions`,
`/accounts/:id/transactions` with to/from data, and
`/liquidity-pools/:id/transactions`.

```sql
CREATE TABLE token_transfers (
    id                 BIGSERIAL,
    transaction_id     BIGINT NOT NULL,
    ledger_sequence    BIGINT NOT NULL,
    transfer_index     SMALLINT NOT NULL,
    asset_type         VARCHAR(20) NOT NULL,
    asset_code         VARCHAR(12),
    asset_issuer       VARCHAR(56) REFERENCES accounts(account_id),
    contract_id        VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    from_account       VARCHAR(56) REFERENCES accounts(account_id),
    from_account_muxed VARCHAR(69),
    to_account         VARCHAR(56) REFERENCES accounts(account_id),
    to_account_muxed   VARCHAR(69),
    amount             NUMERIC(39,0) NOT NULL,
    transfer_type      VARCHAR(20) NOT NULL,
    pool_id            VARCHAR(64) REFERENCES liquidity_pools(pool_id),
    source             VARCHAR(10) NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),
    UNIQUE (transaction_id, transfer_index, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at)
        ON DELETE CASCADE,
    CONSTRAINT ck_tt_source CHECK (source IN ('operation', 'event'))
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_tt_contract ON token_transfers (contract_id, created_at DESC)
    WHERE contract_id IS NOT NULL;
CREATE INDEX idx_tt_asset    ON token_transfers (asset_code, asset_issuer, created_at DESC)
    WHERE asset_code IS NOT NULL;
CREATE INDEX idx_tt_from     ON token_transfers (from_account, created_at DESC)
    WHERE from_account IS NOT NULL;
CREATE INDEX idx_tt_to       ON token_transfers (to_account, created_at DESC)
    WHERE to_account IS NOT NULL;
CREATE INDEX idx_tt_pool     ON token_transfers (pool_id, created_at DESC)
    WHERE pool_id IS NOT NULL;
CREATE INDEX idx_tt_tx       ON token_transfers (transaction_id);
```

No CHECK on `transfer_type` — application-level taxonomy (ADR 0015
policy category D).

---

### 13. `nfts`

Role: NFT registry with current-owner denormalization for list views.

```sql
CREATE TABLE nfts (
    id                    SERIAL PRIMARY KEY,
    contract_id           VARCHAR(56) NOT NULL REFERENCES soroban_contracts(contract_id),
    token_id              VARCHAR(256) NOT NULL,
    collection_name       VARCHAR(256),
    name                  VARCHAR(256),
    media_url             TEXT,
    metadata              JSONB,                          -- SEP-0050: no schema standard
    minted_at_ledger      BIGINT,
    current_owner         VARCHAR(56) REFERENCES accounts(account_id),
    current_owner_ledger  BIGINT,
    UNIQUE (contract_id, token_id)
);
CREATE INDEX idx_nfts_collection ON nfts (contract_id, collection_name)
    WHERE collection_name IS NOT NULL;
CREATE INDEX idx_nfts_owner      ON nfts (current_owner)
    WHERE current_owner IS NOT NULL;
CREATE INDEX idx_nfts_name_trgm  ON nfts USING GIN (name gin_trgm_ops)
    WHERE name IS NOT NULL;
```

---

### 14. `nft_ownership` (partitioned monthly by `created_at`)

Role: full ownership history per NFT.

```sql
CREATE TABLE nft_ownership (
    nft_id          INTEGER NOT NULL REFERENCES nfts(id) ON DELETE CASCADE,
    transaction_id  BIGINT NOT NULL,
    owner_account   VARCHAR(56) REFERENCES accounts(account_id),  -- NULL on burn
    event_type      VARCHAR(20) NOT NULL,
    ledger_sequence BIGINT NOT NULL,
    event_order     SMALLINT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (nft_id, created_at, ledger_sequence, event_order),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at)
        ON DELETE CASCADE,
    CONSTRAINT ck_nft_event_type CHECK (
        event_type IN ('mint', 'transfer', 'burn')
    )
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_nft_own_owner ON nft_ownership (owner_account, created_at DESC)
    WHERE owner_account IS NOT NULL;
```

---

### 15. `liquidity_pools`

Role: classic LP registry (identity + static config). Current reserves
/ TVL live in snapshots, not here.

```sql
CREATE TABLE liquidity_pools (
    pool_id           VARCHAR(64) PRIMARY KEY,
    asset_a_type      VARCHAR(20) NOT NULL,
    asset_a_code      VARCHAR(12),
    asset_a_issuer    VARCHAR(56) REFERENCES accounts(account_id),
    asset_b_type      VARCHAR(20) NOT NULL,
    asset_b_code      VARCHAR(12),
    asset_b_issuer    VARCHAR(56) REFERENCES accounts(account_id),
    fee_bps           INTEGER NOT NULL,
    created_at_ledger BIGINT NOT NULL
);
CREATE INDEX idx_pools_asset_a ON liquidity_pools (asset_a_code, asset_a_issuer)
    WHERE asset_a_code IS NOT NULL;
CREATE INDEX idx_pools_asset_b ON liquidity_pools (asset_b_code, asset_b_issuer)
    WHERE asset_b_code IS NOT NULL;
CREATE INDEX idx_pools_prefix  ON liquidity_pools (pool_id text_pattern_ops);
```

---

### 16. `liquidity_pool_snapshots` (partitioned monthly by `created_at`)

Role: per-change history of pool state. Backs `/liquidity-pools/:id/chart`.

```sql
CREATE TABLE liquidity_pool_snapshots (
    id               BIGSERIAL,
    pool_id          VARCHAR(64) NOT NULL REFERENCES liquidity_pools(pool_id),
    ledger_sequence  BIGINT NOT NULL,
    reserve_a        NUMERIC(39,0) NOT NULL,
    reserve_b        NUMERIC(39,0) NOT NULL,
    total_shares     NUMERIC NOT NULL,
    tvl              NUMERIC,
    volume           NUMERIC,
    fee_revenue      NUMERIC,
    created_at       TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),
    UNIQUE (pool_id, ledger_sequence, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_lps_pool ON liquidity_pool_snapshots (pool_id, created_at DESC);
CREATE INDEX idx_lps_tvl  ON liquidity_pool_snapshots (tvl DESC, created_at DESC)
    WHERE tvl IS NOT NULL;
```

---

### 17. `lp_positions`

Role: current share balances per participant per pool. Backs
"Pool participants" section of `/liquidity-pools/:id`.

```sql
CREATE TABLE lp_positions (
    pool_id              VARCHAR(64) NOT NULL REFERENCES liquidity_pools(pool_id),
    account_id           VARCHAR(56) NOT NULL REFERENCES accounts(account_id),
    shares               NUMERIC(39,0) NOT NULL,
    first_deposit_ledger BIGINT NOT NULL,
    last_updated_ledger  BIGINT NOT NULL,
    PRIMARY KEY (pool_id, account_id)
);
CREATE INDEX idx_lpp_account ON lp_positions (account_id);
CREATE INDEX idx_lpp_shares  ON lp_positions (pool_id, shares DESC)
    WHERE shares > 0;
```

---

### 18. `account_balances_current`

Role: O(1) lookup for current balance per (account, asset). Upsert with
watermark on `last_updated_ledger`.

```sql
CREATE TABLE account_balances_current (
    account_id          VARCHAR(56) NOT NULL REFERENCES accounts(account_id),
    asset_type          VARCHAR(20) NOT NULL,
    asset_code          VARCHAR(12) NOT NULL DEFAULT '',
    issuer              VARCHAR(56) NOT NULL DEFAULT '',
    balance             NUMERIC(39,0) NOT NULL,
    last_updated_ledger BIGINT NOT NULL,
    PRIMARY KEY (account_id, asset_type, asset_code, issuer),
    CONSTRAINT ck_abc_asset_type CHECK (
        asset_type IN ('native', 'credit_alphanum4', 'credit_alphanum12',
                       'pool_share', 'contract')
    )
);
CREATE INDEX idx_abc_asset_balance
    ON account_balances_current (asset_code, issuer, balance DESC)
    WHERE asset_type <> 'native';
```

`issuer` uses empty-string sentinel for native XLM (no FK; deliberate
— see ADR 0013).

---

### 19. `account_balance_history` (partitioned monthly by `created_at`)

Role: per-change balance history per (account, asset). Backs "balance at
ledger N" queries. Append-only.

```sql
CREATE TABLE account_balance_history (
    account_id      VARCHAR(56) NOT NULL REFERENCES accounts(account_id),
    ledger_sequence BIGINT NOT NULL,
    asset_type      VARCHAR(20) NOT NULL,
    asset_code      VARCHAR(12) NOT NULL DEFAULT '',
    issuer          VARCHAR(56) NOT NULL DEFAULT '',
    balance         NUMERIC(39,0) NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (account_id, ledger_sequence, asset_type, asset_code, issuer, created_at),
    CONSTRAINT ck_abh_asset_type CHECK (
        asset_type IN ('native', 'credit_alphanum4', 'credit_alphanum12',
                       'pool_share', 'contract')
    )
) PARTITION BY RANGE (created_at);
```

---

### Summary table — what is in DB, what is on S3

| What                                                              |                      DB                       |              S3              |
| ----------------------------------------------------------------- | :-------------------------------------------: | :--------------------------: |
| Ledger timeline, close metadata                                   |                       ✓                       | via `parsed_ledger_{N}.json` |
| Transaction index rows + filters                                  |                       ✓                       |                              |
| `envelope_xdr`, `result_xdr`, `result_meta_xdr`, `operation_tree` |                                               |              ✓               |
| Operation index row + filter cols                                 |                       ✓                       |                              |
| Operation `details` JSONB                                         |                                               |              ✓               |
| Event index row + `topic0`                                        |                       ✓                       |                              |
| Event `topics`, `data` payload                                    |                                               |              ✓               |
| Invocation index row                                              |                       ✓                       |                              |
| Invocation `function_args`, `return_value`                        |                                               |              ✓               |
| Account state (current)                                           |                       ✓                       |                              |
| Balance history                                                   |                       ✓                       |                              |
| NFT registry + metadata                                           | ✓ (metadata JSONB inline by SEP-0050 absence) |                              |
| NFT ownership history                                             |                       ✓                       |                              |
| LP registry + snapshots + positions                               |                       ✓                       |                              |
| Token registry + classification                                   |                       ✓                       |                              |
| Token metadata                                                    |                                               |              ✓               |
| Contract registry + classification                                |                       ✓                       |                              |
| Contract metadata, WASM spec                                      |                                               |              ✓               |
| Transfer ledger (unified classic + Soroban)                       |                       ✓                       |                              |
| Signatures                                                        |                                               |    ✓ (via `envelope_xdr`)    |

---

## Open questions

None. All corrections from the 0014–0017 sequence are resolved.

Two deferred topics remain (unchanged from prior ADRs, not in scope
here):

- Retirement of `wasm_interface_metadata` after task 0118's parser-level
  WASM cache is in place.
- Inclusion of Soroban-native AMM pools (Soroswap, Phoenix) in
  `liquidity_pools`, pending a product-level decision.

Both are follow-up ADRs if and when their triggers land.

---

## References

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0012: Lightweight bridge DB schema revision](0012_lightweight-bridge-db-schema-revision.md)
- [ADR 0013: Sequential ingest schema with full FK integrity](0013_sequential-ingest-full-fk-schema.md)
- [ADR 0014: Schema fixes — Stellar/XDR compliance](0014_schema-fixes-stellar-xdr-compliance.md)
- [ADR 0015: Hash index, typed topic0, migration honesty, CHECK policy](0015_hash-index-topic-typing-migration-honesty.md)
- [ADR 0016: Hash fail-fast, topic0 pre-GA unification, filter contract](0016_hash-fail-fast-topic-unification-filter-contract.md)
- [PostgreSQL: Partitioning](https://www.postgresql.org/docs/current/ddl-partitioning.html)
- [PostgreSQL: CHECK constraints](https://www.postgresql.org/docs/current/ddl-constraints.html)
- [CAP-0067: Unified Events](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0067.md)
- [SEP-0023: Muxed Accounts](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0023.md)
- [SEP-0041: Soroban Token Standard](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md)
- [SEP-0050: Non-Fungible Tokens](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0050.md)
