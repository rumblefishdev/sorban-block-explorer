---
id: '0019'
title: 'DB schema: tokens and accounts tables'
type: FEATURE
status: completed
related_adr: ['0005']
related_tasks: ['0011', '0015', '0092']
tags: [priority-medium, effort-small, layer-database]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005 + research 0092: plain SQL migrations instead of Drizzle ORM'
  - date: 2026-04-02
    status: active
    who: stkrolikiewicz
    note: 'Activated for implementation'
  - date: 2026-04-02
    status: completed
    who: stkrolikiewicz
    note: >
      Aligned migration 0005 with spec: NUMERIC(28,7) for total_supply,
      DEFAULT 0 for holder_count. Updated task spec to match actual design
      (partial unique indexes, no FK to ledgers). Verified on fresh Postgres.
      All 9 acceptance criteria met.
---

# DB schema: tokens and accounts tables

## Summary

Implement the SQL DDL for the `tokens` and `accounts` tables. These are derived, query-oriented explorer entities that unify classic Stellar assets with Soroban token contracts and provide account summary data for explorer views.

## Status: Completed

**Current state:** Migration 0005 finalized. All acceptance criteria met. Verified on fresh PostgreSQL.

## Context

Tokens and accounts are derived-state tables. They are not populated directly from raw ledger ingestion but are upserted from extracted state and known event patterns. Their data must be kept current through ledger-sequence watermark guards to prevent older backfill data from overwriting newer live state.

### Full DDL

#### tokens

```sql
CREATE TABLE tokens (
    id               SERIAL PRIMARY KEY,
    asset_type       VARCHAR(20) NOT NULL CHECK (asset_type IN ('classic', 'sac', 'soroban')),
    asset_code       VARCHAR(12),
    issuer_address   VARCHAR(56),
    contract_id      VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    name             VARCHAR(256),
    total_supply     NUMERIC(28, 7),
    holder_count     INTEGER DEFAULT 0,
    metadata         JSONB
);

-- Partial unique indexes (superior to simple UNIQUE constraints -- handle NULL correctly per asset_type)
CREATE UNIQUE INDEX idx_tokens_classic ON tokens (asset_code, issuer_address) WHERE asset_type IN ('classic', 'sac');
CREATE UNIQUE INDEX idx_tokens_soroban ON tokens (contract_id) WHERE asset_type = 'soroban';
CREATE UNIQUE INDEX idx_tokens_sac ON tokens (contract_id) WHERE asset_type = 'sac';
CREATE INDEX idx_tokens_type ON tokens (asset_type);
```

#### accounts

```sql
CREATE TABLE accounts (
    account_id         VARCHAR(56) PRIMARY KEY,
    first_seen_ledger  BIGINT NOT NULL,
    last_seen_ledger   BIGINT NOT NULL,
    sequence_number    BIGINT NOT NULL,
    balances           JSONB NOT NULL DEFAULT '[]'::jsonb,
    home_domain        VARCHAR(256)
);

CREATE INDEX idx_accounts_last_seen ON accounts (last_seen_ledger DESC);
```

> **No FK to ledgers** -- intentional. During concurrent backfill + live ingestion, an account row may reference a ledger not yet ingested. All derived-state tables (nfts, liquidity_pools) follow this same pattern.

### Design Notes

#### tokens

- **CHECK constraint**: `asset_type IN ('classic', 'sac', 'soroban')` enforces that every token row is classified into one of three valid types:
  - `classic` -- traditional Stellar asset identified by asset_code + issuer_address
  - `sac` -- Stellar Asset Contract (classic asset wrapped in a Soroban contract)
  - `soroban` -- native Soroban token contract
- **Partial unique indexes** scoped by `asset_type` (superior to simple UNIQUE constraints):
  - `idx_tokens_classic` on `(asset_code, issuer_address) WHERE asset_type IN ('classic', 'sac')` -- classic identity pair
  - `idx_tokens_soroban` on `(contract_id) WHERE asset_type = 'soroban'` -- Soroban contract identity
  - `idx_tokens_sac` on `(contract_id) WHERE asset_type = 'sac'` -- SAC contract identity
  - Partial indexes handle NULL correctly: two classic tokens with NULL contract_id don't conflict on the soroban index
- **FK: contract_id -> soroban_contracts(contract_id)** -- links Soroban-backed tokens to their contract entity for metadata, interface, and activity lookups.
- **total_supply** uses NUMERIC(28, 7) for precision with Stellar's 7-decimal-place amounts.
- **metadata** is JSONB for flexible token metadata that varies by token type and issuer.

#### accounts

- **No FK to ledgers**: `first_seen_ledger` and `last_seen_ledger` are `BIGINT NOT NULL` without foreign keys. This is intentional -- during concurrent backfill + live ingestion, an account may reference a ledger not yet ingested. All derived-state tables follow this pattern.
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

> **Migration approach:** Plain SQL (per ADR 0005). Run via psql or sqlx migrate run.

### Step 1: SQL DDL for tokens

Define the table with CHECK constraint on asset_type, FK to soroban_contracts, and partial unique indexes scoped by asset_type.

### Step 2: SQL DDL for accounts

Define the table with BIGINT NOT NULL ledger columns (no FK to ledgers -- backfill safety), JSONB default for balances, and last_seen_ledger DESC index.

### Step 3: Migration file

Migration 0005 updated in place (not yet deployed). Verify with `cargo check -p db`.

### Step 4: Validate unique constraints

Test that:

- Two classic tokens with the same (asset_code, issuer_address) pair are rejected.
- Two Soroban tokens with the same contract_id are rejected.
- A classic token with NULL contract_id and a Soroban token with NULL asset_code/issuer can coexist.

### Step 5: Watermark upsert pattern

Already implemented in `soroban.rs` (`upsert_account_state`, `upsert_token`). Formally owned by task 0028.

## Acceptance Criteria

- [x] SQL DDL for tokens with CHECK constraint on asset_type
- [x] Partial unique indexes enforced per asset_type (classic/sac, soroban, sac)
- [x] FK from tokens.contract_id to soroban_contracts.contract_id
- [x] SQL DDL for accounts with BIGINT NOT NULL ledger columns (no FK -- intentional for backfill safety)
- [x] balances column defaults to '[]'::jsonb
- [x] idx_accounts_last_seen index on accounts.last_seen_ledger DESC
- [x] total_supply uses NUMERIC(28, 7) for Stellar precision
- [x] holder_count defaults to 0
- [x] Migration applies cleanly to a fresh PostgreSQL instance

> **Note:** Watermark-guarded upsert logic is already implemented in `soroban.rs` (`upsert_account_state`, `upsert_token`). Testing is scope of task 0028.

## Implementation Notes

**Files changed:** 1 migration file (`crates/db/migrations/0005_create_accounts_tokens.sql`) — 2 line changes.

Migration 0005 already existed with both tables from earlier scaffolding work. This task aligned the DDL with the spec and documented the design rationale.

## Design Decisions

### From Plan

1. **NUMERIC(28, 7) for total_supply**: Enforces Stellar's 7-decimal-place precision at the DB level. Prevents silent precision drift from extractors.

2. **DEFAULT 0 for holder_count**: Sensible zero-state for newly discovered tokens.

### Emerged

3. **No FK from accounts to ledgers**: Original spec called for `REFERENCES ledgers(sequence)` on both `first_seen_ledger` and `last_seen_ledger`. Dropped because concurrent backfill + live ingestion means an account may reference a ledger not yet ingested. All derived-state tables (nfts in 0006, liquidity_pools in 0006) follow the same no-FK pattern — this is a project-wide convention, not an oversight.

4. **Partial unique indexes instead of simple UNIQUE constraints**: Original spec had `UNIQUE(asset_code, issuer_address)` and `UNIQUE(contract_id)`. Kept existing partial indexes scoped by `asset_type` — they handle NULL correctly and prevent cross-type conflicts.

5. **VARCHAR(20) for asset_type, VARCHAR(256) for name**: Wider than original spec (VARCHAR(10), VARCHAR(100)). Kept existing values for safety margin — no functional difference.

## Issues Encountered

None. Migration was already scaffolded; changes were minimal.

## Notes

- These tables are unpartitioned. They represent derived explorer state and are expected to be smaller than the transaction-centric tables.
- The watermark upsert pattern should be implemented as a reusable utility since it applies to multiple derived-state tables (tokens, accounts, nfts, liquidity_pools).
- The accounts table scope is intentionally limited to summary, balances, and recent activity. Richer account state should only be added if the product scope expands.
