---
id: '0136'
title: 'DB: add surrogate BIGSERIAL IDs to all tables using string PKs'
type: REFACTOR
status: backlog
related_adr: []
related_tasks: ['0121', '0122', '0126']
tags: [priority-high, effort-large, layer-db]
milestone: 1
links:
  - crates/db/migrations/
  - crates/db/src/soroban.rs
history:
  - date: '2026-04-13'
    status: backlog
    who: fmazur
    note: 'Created per senior recommendation — all tables should use auto-incremented integer IDs for maximum DB performance.'
  - date: '2026-04-13'
    status: active
    who: fmazur
    note: 'Activated task for implementation.'
  - date: '2026-04-14'
    status: backlog
    who: fmazur
    note: 'Moved back to backlog — deprioritized.'
---

# DB: add surrogate BIGSERIAL IDs to all tables using string PKs

## Summary

6 tables currently use VARCHAR string primary keys (on-chain identifiers). String PKs are
significantly slower than integer PKs for JOINs, FK lookups, index scans, and ON CONFLICT
checks. Adding surrogate `BIGSERIAL` IDs and converting FK references to integer will
improve query performance across the board — especially at mainnet scale with millions of
rows.

## Context

Senior recommendation from daily (2026-04-13): use auto-incremented integer IDs everywhere
possible for maximum DB operation speed.

**Performance impact of string vs integer PKs:**

- B-tree index on BIGINT: 8 bytes per key, single CPU instruction comparison
- B-tree index on VARCHAR(56): 56+ bytes per key, byte-by-byte comparison with collation
- Index size reduction: ~3-4x smaller indexes → more data fits in RAM → fewer disk reads
- JOIN/FK lookup: integer comparison is O(1), string comparison is O(n)

## Tables to change

| Table                       | Current PK                          | New PK         | Keep old key as                                          |
| --------------------------- | ----------------------------------- | -------------- | -------------------------------------------------------- |
| **ledgers**                 | `sequence BIGINT`                   | `id BIGSERIAL` | `UNIQUE NOT NULL` (needed for idempotency)               |
| **soroban_contracts**       | `contract_id VARCHAR(56)`           | `id BIGSERIAL` | `UNIQUE NOT NULL` (needed for ON CONFLICT)               |
| **accounts**                | `account_id VARCHAR(56)`            | `id BIGSERIAL` | `UNIQUE NOT NULL` (needed for ON CONFLICT)               |
| **liquidity_pools**         | `pool_id VARCHAR(64)`               | `id BIGSERIAL` | `UNIQUE NOT NULL` (needed for ON CONFLICT)               |
| **nfts**                    | `(contract_id, token_id)` composite | `id BIGSERIAL` | `UNIQUE(contract_id, token_id)` (needed for ON CONFLICT) |
| **wasm_interface_metadata** | `wasm_hash VARCHAR(64)`             | `id BIGSERIAL` | `UNIQUE NOT NULL` (needed for UPSERT)                    |

## FK references to update

These columns currently reference string PKs and must be converted to integer FKs:

| Table                    | Column      | Currently references          | Change to                         |
| ------------------------ | ----------- | ----------------------------- | --------------------------------- |
| soroban_events           | contract_id | soroban_contracts.contract_id | integer FK → soroban_contracts.id |
| soroban_invocations      | contract_id | soroban_contracts.contract_id | integer FK → soroban_contracts.id |
| nfts                     | contract_id | soroban_contracts.contract_id | integer FK → soroban_contracts.id |
| tokens                   | contract_id | soroban_contracts.contract_id | integer FK → soroban_contracts.id |
| liquidity_pool_snapshots | pool_id     | liquidity_pools.pool_id       | integer FK → liquidity_pools.id   |

## Implementation

### Step 1: Migration — add surrogate ID columns

Add `id BIGSERIAL` to each table as new PK. Keep existing string columns as `UNIQUE NOT NULL`.

```sql
-- Example for soroban_contracts:
ALTER TABLE soroban_contracts ADD COLUMN id BIGSERIAL;
ALTER TABLE soroban_contracts DROP CONSTRAINT soroban_contracts_pkey;
ALTER TABLE soroban_contracts ADD PRIMARY KEY (id);
ALTER TABLE soroban_contracts ADD UNIQUE (contract_id);
```

**Order matters** — parent tables first (ledgers, accounts, soroban_contracts, liquidity_pools),
then children that reference them.

### Step 2: Migration — add integer FK columns to child tables

For each FK reference, add a new integer column, populate it via JOIN, then drop the old
string FK column.

```sql
-- Example for soroban_events:
ALTER TABLE soroban_events ADD COLUMN contract_int_id BIGINT;
UPDATE soroban_events e SET contract_int_id = c.id
  FROM soroban_contracts c WHERE e.contract_id = c.contract_id;
ALTER TABLE soroban_events DROP COLUMN contract_id;
ALTER TABLE soroban_events RENAME COLUMN contract_int_id TO contract_id;
ALTER TABLE soroban_events ADD FOREIGN KEY (contract_id) REFERENCES soroban_contracts(id);
```

### Step 3: Update Rust code

- Update all INSERT/SELECT/JOIN queries in `crates/db/src/` to use integer IDs
- Update `crates/xdr-parser/src/types.rs` structs if needed
- Update `crates/indexer/src/handler/convert.rs` to resolve string→integer via lookup
- Update `crates/api/src/` to translate between external string IDs and internal integer IDs

### Step 4: Update indexes

Rebuild indexes that referenced string columns to use integer columns instead.

## Impact on other tasks

Tasks that add new tables or columns should use integer FKs from the start:

| Task                         | Impact                                                                   |
| ---------------------------- | ------------------------------------------------------------------------ |
| **0121** (nft_transfers)     | New table should FK to `nfts.id` (integer) not composite string PK       |
| **0122** (signatures)        | No impact — adds JSONB column to transactions (already has BIGSERIAL)    |
| **0126** (pool_participants) | New table should FK to `liquidity_pools.id` (integer) not string pool_id |

**Recommendation:** Complete 0136 before 0121 and 0126 to avoid creating new string FKs
that would need immediate migration.

## Acceptance Criteria

- [ ] All 6 tables have `id BIGSERIAL PRIMARY KEY`
- [ ] Original string identifiers preserved as `UNIQUE NOT NULL` columns
- [ ] All FK references converted from VARCHAR to integer
- [ ] All existing indexes rebuilt on integer columns where applicable
- [ ] ON CONFLICT / idempotency logic still works (uses UNIQUE constraint on string column)
- [ ] All Rust code updated — inserts, selects, joins use integer IDs
- [ ] API layer translates external string IDs ↔ internal integer IDs transparently
- [ ] All existing tests pass
- [ ] Migration is reversible (down migration restores string PKs)
- [ ] Benchmark: demonstrate measurable improvement on JOIN/lookup queries with >10K rows
