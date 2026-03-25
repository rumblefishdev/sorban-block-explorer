---
id: '0018'
title: 'DB schema: Soroban tables (contracts, invocations, events, interpretations)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0016', '0010']
tags: [priority-high, effort-medium, layer-database]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# DB schema: Soroban tables (contracts, invocations, events, interpretations)

## Summary

Implement the Drizzle ORM schema definitions and SQL DDL for the four Soroban-specific tables: `soroban_contracts`, `soroban_invocations`, `soroban_events`, and `event_interpretations`. These tables model Soroban contract activity as first-class explorer entities with decoded, queryable data.

## Status: Backlog

**Current state:** Not started.

## Context

The Soroban tables form the contract-centric activity model of the explorer. Contracts are top-level entities; invocations and events are transaction children that also reference contracts; interpretations enrich events with human-readable summaries.

### Full DDL

#### soroban_contracts

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

#### soroban_invocations

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

#### soroban_events

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

#### event_interpretations

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

### Design Notes

#### soroban_contracts

- **contract_id** (VARCHAR(56)) is the public stable identifier and primary key.
- **contract_type** values: `'token'`, `'dex'`, `'lending'`, `'nft'`, `'other'`. Used for explorer-level classification.
- **metadata** is JSONB because contract metadata quality and shape varies. It may include optional extracted interface signatures for the contract.
- **search_vector** is a TSVECTOR column defined as `GENERATED ALWAYS AS (to_tsvector('english', coalesce(metadata->>'name', ''))) STORED`. This is a generated column that automatically updates when metadata changes. The GIN index on search_vector enables efficient full-text search on contract names.
- **is_sac** indicates whether the contract is a Stellar Asset Contract (classic asset wrapped in Soroban).

#### soroban_invocations

- **Dual FK**: references both `transactions(id)` with ON DELETE CASCADE and `soroban_contracts(contract_id)` without cascade. Deleting a transaction cascades to remove its invocations. Deleting a contract does NOT cascade (contracts are long-lived entities).
- **Monthly partitioned** by `created_at` using PARTITION BY RANGE.
- **function_args** and **return_value** are JSONB columns containing decoded ScVal values. The shape varies per contract function, so JSONB is the appropriate storage type.
- **ledger_sequence** keeps ledger ordering explicit even where timestamps are the primary access pattern.

#### soroban_events

- **Dual FK**: same pattern as invocations -- CASCADE from transactions, non-cascading reference to contracts.
- **event_type** values: `'contract'`, `'system'`, `'diagnostic'`. Distinguishes the three CAP-67 event classes.
- **topics** is JSONB NOT NULL with a GIN index for pattern matching and event signature queries.
- **data** is JSONB NOT NULL containing the decoded event payload.
- **Monthly partitioned** by `created_at` using PARTITION BY RANGE.

#### event_interpretations

- **FK to soroban_events(id) with ON DELETE CASCADE**. Deleting an event cascades to remove its interpretations.
- **interpretation_type** values: `'swap'`, `'transfer'`, `'mint'`, `'burn'`. These are the known event patterns the Event Interpreter Lambda recognizes.
- This is an **enrichment table**, not canonical chain truth. It is written by the Event Interpreter Lambda which runs every 5 minutes via EventBridge, post-processing recent events into human-readable summaries.
- **structured_data** keeps normalized interpretation payloads queryable and extensible.

### Cascade Chain

```
DELETE transaction
  -> CASCADE to soroban_invocations (via transaction_id FK)
  -> CASCADE to soroban_events (via transaction_id FK)
       -> CASCADE to event_interpretations (via event_id FK)
```

Deleting a transaction removes all its invocations, events, and the interpretations of those events.

## Implementation Plan

### Step 1: Drizzle schema for soroban_contracts

Define the table with all columns, primary key, FK to ledgers, generated TSVECTOR column, and both indexes (contract_type and GIN on search_vector).

### Step 2: Drizzle schema for soroban_invocations

Define the partitioned table with dual FKs (transaction CASCADE, contract non-cascade), all columns, and both indexes. Configure PARTITION BY RANGE (created_at).

### Step 3: Drizzle schema for soroban_events

Define the partitioned table with dual FKs, all columns, and both indexes (contract + created_at DESC, GIN on topics). Configure PARTITION BY RANGE (created_at).

### Step 4: Drizzle schema for event_interpretations

Define the table with FK to soroban_events (CASCADE), all columns, and the interpretation_type index.

### Step 5: Generate migrations

Use Drizzle Kit to generate migration files. Supplement with raw SQL for partitioning clauses and the GENERATED ALWAYS AS column if Drizzle does not natively support them.

### Step 6: Create initial monthly partitions

Create initial partitions for soroban_invocations and soroban_events covering at least the next 3 months. Partition naming: `{table}_y{YYYY}m{MM}`.

### Step 7: Validate cascade chain

Test the full cascade: delete a transaction and verify invocations, events, and interpretations are all removed.

### Step 8: Validate search_vector

Test that inserting a contract with metadata containing a name field populates the search_vector and that full-text queries return expected results.

## Acceptance Criteria

- [ ] Drizzle schema for soroban_contracts matches DDL with generated TSVECTOR column
- [ ] Drizzle schema for soroban_invocations matches DDL with monthly partitioning
- [ ] Drizzle schema for soroban_events matches DDL with monthly partitioning
- [ ] Drizzle schema for event_interpretations matches DDL with CASCADE from events
- [ ] All indexes are created correctly (including GIN indexes)
- [ ] Cascade chain works: delete transaction removes invocations, events, and interpretations
- [ ] search_vector is automatically populated from metadata->>'name'
- [ ] Full-text search queries work against the GIN-indexed search_vector
- [ ] Initial monthly partitions are created for invocations and events
- [ ] Migration files apply cleanly to a fresh PostgreSQL instance

## Notes

- The GENERATED ALWAYS AS column for search_vector may require raw SQL in the migration if Drizzle ORM does not support generated columns natively.
- Monthly partition creation for invocations and events is covered more comprehensively in task 0022 (partition management automation). This task should create the initial set.
- The Event Interpreter Lambda (which populates event_interpretations) is a separate infrastructure concern. This task only defines the storage schema.
