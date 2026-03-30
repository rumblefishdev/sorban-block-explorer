---
id: '0016'
title: 'DB schema: ledgers and transactions tables'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0015', '0009']
tags: [priority-high, effort-medium, layer-database]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-30
    status: active
    who: stkrolikiewicz
    note: 'Promoted to active'
---

# DB schema: ledgers and transactions tables

## Summary

Implement the Drizzle ORM schema definitions and corresponding SQL DDL for the two backbone tables of the block explorer: `ledgers` and `transactions`. These tables form the core timeline that all other explorer entities reference.

## Status: Backlog

**Current state:** Not started.

## Context

The ledgers and transactions tables are the foundation of the explorer data model. Ledgers represent the canonical ledger-close timeline, and transactions are the primary activity entity for browsing, detail views, and child-entity anchoring.

### Relationship Graph

```
ledgers (sequence PK)
  └── transactions (ledger_sequence FK)
        ├── operations (transaction_id FK, CASCADE)
        ├── soroban_invocations (transaction_id FK, CASCADE)
        └── soroban_events (transaction_id FK, CASCADE)
```

### Full DDL

#### ledgers

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

#### transactions

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

### Design Notes

- **id** is an internal surrogate key used for child table foreign keys (operations, invocations, events). It is NOT the public lookup key.
- **hash** is the main public lookup key for transaction detail routes. It is UNIQUE and indexed.
- **envelope_xdr** and **result_xdr** are NOT NULL -- these raw XDR payloads are always preserved for advanced decode and debugging.
- **result_meta_xdr** IS nullable -- preserved when available for advanced decode/debug recovery paths, but not guaranteed for all transactions.
- **operation_tree** stores the decoded invocation hierarchy as JSONB, enabling transaction-detail tree rendering without reparsing result meta on every request.
- **parse_error** (BOOLEAN DEFAULT FALSE) allows partial retention of transaction records even when full XDR decode fails. This ensures the explorer does not silently drop transactions.
- **ledger_sequence** links each transaction to the ledger timeline via FK to ledgers(sequence).

### Index Purposes

| Index                        | Purpose                                                                |
| ---------------------------- | ---------------------------------------------------------------------- |
| `ledgers.hash` (UNIQUE)      | Ledger lookup by hash (secondary to sequence-based navigation)         |
| `idx_closed_at`              | Recent-ledger browsing, freshness comparisons, DESC ordering           |
| `transactions.hash` (UNIQUE) | Primary public lookup for transaction detail routes                    |
| `idx_source`                 | Account-centric transaction history (source_account + created_at DESC) |
| `idx_ledger`                 | Efficient join from ledger detail to its transactions                  |

### Write Patterns

- **Append-oriented**: ledger and transaction records are inserted in per-ledger database transactions. One ledger close produces one ledger row and N transaction rows.
- **Replay-safe dedup**: ingestion uses the ledger sequence as a deduplication key. Re-processing the same ledger replaces or skips existing rows rather than creating duplicates.
- **Batch child rows**: operations, invocations, and events for a ledger's transactions are batch-inserted within the same DB transaction.

## Implementation Plan

### Step 1: Drizzle schema definition for ledgers

Define the ledgers table using Drizzle ORM schema builder in the shared database library. Include all columns, the primary key, unique constraint on hash, and the closed_at DESC index.

### Step 2: Drizzle schema definition for transactions

Define the transactions table with all columns, the BIGSERIAL primary key, unique constraint on hash, foreign key to ledgers(sequence), and all three indexes.

### Step 3: Generate migration

Use Drizzle Kit to generate the SQL migration file from the schema definitions. Verify the generated DDL matches the architecture specification.

### Step 4: Validate relationships

Ensure the FK from transactions.ledger_sequence to ledgers.sequence is correctly defined and that the relationship is queryable through Drizzle's relational API.

### Step 5: Test with local PostgreSQL

Run the migration against local PostgreSQL and verify table creation, index creation, and basic insert/select operations.

## Acceptance Criteria

- [ ] Drizzle schema for ledgers table matches the DDL specification exactly
- [ ] Drizzle schema for transactions table matches the DDL specification exactly
- [ ] All indexes are created with correct columns and ordering
- [ ] FK from transactions.ledger_sequence to ledgers.sequence is enforced
- [ ] result_meta_xdr is nullable; envelope_xdr and result_xdr are NOT NULL
- [ ] operation_tree is JSONB and nullable
- [ ] parse_error defaults to FALSE
- [ ] Migration file is generated and applies cleanly to a fresh PostgreSQL instance
- [ ] Basic insert and query operations work through Drizzle ORM

## Notes

- These two tables are unpartitioned. They represent the core timeline and are expected to grow large but remain manageable with proper indexing.
- Child tables (operations, invocations, events) are defined in tasks 0017 and 0018.
- The transactions table is the most-referenced parent in the schema. Its id column is the FK target for operations, soroban_invocations, and soroban_events.
