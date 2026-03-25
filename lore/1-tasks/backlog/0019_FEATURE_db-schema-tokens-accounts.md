---
id: '0019'
title: 'DB schema: tokens and accounts tables'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0011']
tags: [priority-medium, effort-small, layer-database]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# DB schema: tokens and accounts tables

## Summary

Implement the Drizzle ORM schema definitions and SQL DDL for the `tokens` and `accounts` tables. These are derived, query-oriented explorer entities that unify classic Stellar assets with Soroban token contracts and provide account summary data for explorer views.

## Status: Backlog

**Current state:** Not started.

## Context

Tokens and accounts are derived-state tables. They are not populated directly from raw ledger ingestion but are upserted from extracted state and known event patterns. Their data must be kept current through ledger-sequence watermark guards to prevent older backfill data from overwriting newer live state.

### Full DDL

#### tokens

```sql
CREATE TABLE tokens (
    id               SERIAL PRIMARY KEY,
    asset_type       VARCHAR(10) NOT NULL CHECK (asset_type IN ('classic', 'sac', 'soroban')),
    asset_code       VARCHAR(12),
    issuer_address   VARCHAR(56),
    contract_id      VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    name             VARCHAR(100),
    total_supply     NUMERIC(28, 7),
    holder_count     INT DEFAULT 0,
    metadata         JSONB,
    UNIQUE (asset_code, issuer_address),
    UNIQUE (contract_id)
);
```

#### accounts

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

### Design Notes

#### tokens

- **CHECK constraint**: `asset_type IN ('classic', 'sac', 'soroban')` enforces that every token row is classified into one of three valid types:
  - `classic` -- traditional Stellar asset identified by asset_code + issuer_address
  - `sac` -- Stellar Asset Contract (classic asset wrapped in a Soroban contract)
  - `soroban` -- native Soroban token contract
- **Two unique constraints** serving different token identification patterns:
  - `UNIQUE(asset_code, issuer_address)` -- for classic assets, the canonical identity pair
  - `UNIQUE(contract_id)` -- for Soroban-backed tokens, the contract address is the identity
- **FK: contract_id -> soroban_contracts(contract_id)** -- links Soroban-backed tokens to their contract entity for metadata, interface, and activity lookups.
- **total_supply** uses NUMERIC(28, 7) for precision with Stellar's 7-decimal-place amounts.
- **metadata** is JSONB for flexible token metadata that varies by token type and issuer.

#### accounts

- **Two FKs to ledgers**: `first_seen_ledger` and `last_seen_ledger` both reference `ledgers(sequence)`. These track when the account was first observed and most recently active.
- **balances** is `JSONB NOT NULL DEFAULT '[]'::jsonb` -- an array of balance objects. JSONB is used because an account may hold multiple assets of different types (classic, SAC, Soroban).
- **idx_last_seen** (last_seen_ledger DESC) supports queries for recently active accounts.
- **sequence_number** tracks the account's transaction sequence for display purposes.

### Write Pattern: Derived-State Upserts with Watermarks

Both tables use an upsert pattern guarded by ledger-sequence watermarks:

- When upserting a token or account record, the write includes the ledger sequence at which the state was observed.
- The upsert condition checks that the incoming ledger sequence is >= the currently stored watermark.
- This prevents older backfill data from overwriting newer live state.
- Example: if live ingestion has updated an account at ledger 1000, a backfill write from ledger 500 MUST NOT overwrite the account's balances.

This pattern is critical for correctness when live ingestion and historical backfill run concurrently.

## Implementation Plan

### Step 1: Drizzle schema for tokens

Define the table with all columns, CHECK constraint on asset_type, FK to soroban_contracts, and both UNIQUE constraints.

### Step 2: Drizzle schema for accounts

Define the table with all columns, two FKs to ledgers(sequence), JSONB default for balances, and the last_seen_ledger DESC index.

### Step 3: Generate migration

Use Drizzle Kit to generate the migration file. Verify CHECK constraint and UNIQUE constraints are correctly represented.

### Step 4: Validate unique constraints

Test that:

- Two classic tokens with the same (asset_code, issuer_address) pair are rejected.
- Two Soroban tokens with the same contract_id are rejected.
- A classic token with NULL contract_id and a Soroban token with NULL asset_code/issuer can coexist.

### Step 5: Validate watermark upsert pattern

Implement and test the upsert logic that respects ledger-sequence watermarks, ensuring older writes do not overwrite newer state.

## Acceptance Criteria

- [ ] Drizzle schema for tokens matches DDL with CHECK constraint on asset_type
- [ ] Both UNIQUE constraints (asset_code+issuer_address and contract_id) are enforced
- [ ] FK from tokens.contract_id to soroban_contracts.contract_id is defined
- [ ] Drizzle schema for accounts matches DDL with two FKs to ledgers
- [ ] balances column defaults to '[]'::jsonb
- [ ] idx_last_seen index is created on accounts.last_seen_ledger DESC
- [ ] Watermark-guarded upsert logic prevents older data from overwriting newer state
- [ ] Migration applies cleanly to a fresh PostgreSQL instance

## Notes

- These tables are unpartitioned. They represent derived explorer state and are expected to be smaller than the transaction-centric tables.
- The watermark upsert pattern should be implemented as a reusable utility since it applies to multiple derived-state tables (tokens, accounts, nfts, liquidity_pools).
- The accounts table scope is intentionally limited to summary, balances, and recent activity. Richer account state should only be added if the product scope expands.
