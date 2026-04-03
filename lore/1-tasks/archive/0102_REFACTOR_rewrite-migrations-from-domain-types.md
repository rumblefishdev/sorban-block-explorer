---
id: '0102'
title: 'Rewrite ALL SQL migrations to derive from domain types'
type: REFACTOR
status: completed
related_adr: ['0005']
related_tasks: ['0101', '0018', '0019', '0020']
tags: [priority-high, effort-medium, layer-database, rust]
milestone: 1
links: []
history:
  - date: 2026-04-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0101 future work. DB is empty — no risk.
      Migrations 0002-0006 were created before Rust domain types existed.
      Now that domain types are the source of truth (task 0101), migrations
      should be rewritten to derive from them. Also fixes divergence between
      task 0020 spec and actual migration 0006.
  - date: 2026-04-03
    status: active
    who: stkrolikiewicz
    note: Activated task for implementation.
  - date: 2026-04-03
    status: completed
    who: stkrolikiewicz
    note: >
      Rewrote all 6 migrations (0001-0006) from domain types.
      Added operation_tree to Transaction struct. Dropped stale DEFAULTs
      (parse_error, holder_count); kept is_sac DEFAULT for upsert safety.
      Verified on staging. Closed task 0020. PR #63 merged.
---

# Rewrite ALL SQL migrations to derive from domain types

## Summary

Rewrite all migrations (0001-0006) with domain types (`crates/domain/`) as source of truth. The DB is empty — no data risk. This establishes the correct workflow going forward: types define the contract, migrations implement it.

Scope expanded from original plan: migration 0001 included for consistent style, and `operation_tree` field added to `Transaction` domain struct.

## Status: Completed

**Current state:** Merged to develop (PR #63). Verified on staging.

## Context

All migrations were written before Rust domain types existed (the TS→Rust migration gap). Task 0101 created domain types by reverse-engineering the existing migrations. Now we invert the relationship: domain types are the spec, migrations are derived.

Known issues with current migrations:

- Task 0020 DDL spec diverges from actual migration 0006 (nfts PK structure, column sizes, nullability, fee_bps NOT NULL)
- No single source of truth — changes to schema require updating both domain types and migrations independently
- Migration 0001 was ORM-generated (quoted identifiers, verbose FK syntax) while 0002-0006 were hand-written — inconsistent style
- `transactions.operation_tree` column existed in DB but was missing from `Transaction` domain struct
- Stale `DEFAULT` values on nullable columns (`parse_error`, `is_sac`, `holder_count`) conflicted with `Option<_>` domain types

### Target workflow (going forward)

1. Define/update struct in `crates/domain/`
2. Write migration SQL that matches the struct exactly
3. Both in the same PR

## Implementation Plan

### Step 1: Add operation_tree to Transaction struct

Add `pub operation_tree: Option<serde_json::Value>` to `Transaction` in `crates/domain/src/transaction.rs`. This was a DB column missing from the domain type — pre-computed Soroban invocation call tree populated by the ingestion pipeline.

### Step 2: Delete ALL migrations 0001-0006

Move to `.trash/` per project policy.

### Step 3: Rewrite migration for ledgers + transactions

Derive DDL from `domain::ledger::Ledger` and `domain::transaction::Transaction`. Rewrite from ORM-generated style to clean hand-written SQL. Drop stale `DEFAULT false` from `parse_error`. Include `operation_tree` JSONB column.

### Step 4: Rewrite migration for operations

Derive DDL from `domain::operation::Operation`. Verify every field, type, nullability matches.

### Step 5: Rewrite migration for soroban_contracts

Derive from `domain::soroban::SorobanContract`. Drop `DEFAULT FALSE` from `is_sac`. Include `search_vector` TSVECTOR generated column (DB-only, excluded from domain struct by design).

### Step 6: Rewrite migration for soroban_invocations + soroban_events

Derive from `domain::soroban::SorobanInvocation` and `domain::soroban::SorobanEvent`. Include partitioning and initial partitions.

### Step 7: Rewrite migration for accounts + tokens

Derive from `domain::account::Account` and `domain::token::Token`. Drop `DEFAULT 0` from `holder_count`.

### Step 8: Rewrite migration for nfts + pools + snapshots

Derive from `domain::nft::Nft`, `domain::pool::LiquidityPool`, `domain::pool::LiquidityPoolSnapshot`. Include partitioning for snapshots.

### Step 9: Verify migrations apply cleanly

`sqlx migrate run` against a fresh PostgreSQL instance.

### Step 10: Close/update tasks 0018, 0019, 0020

Update status of DDL tasks to reflect the rewrite.

## Acceptance Criteria

- [x] ALL migrations 0001-0006 rewritten — every column matches domain struct field
- [x] Domain types are the documented source of truth for schema (inline comments)
- [x] `sqlx migrate run` applies cleanly to fresh PostgreSQL (verified on staging)
- [x] Indexes, FKs, constraints, partitioning preserved
- [x] DB-only columns (search_vector) documented as exceptions
- [x] `operation_tree` added to Transaction domain struct
- [x] Stale DEFAULTs removed (parse_error, holder_count); is_sac DEFAULT kept for upsert safety
- [x] Consistent SQL style across all migrations
- [x] Tasks 0018, 0019 already archived; 0020 completed and archived

## Notes

- DB is empty — this is a pure rewrite with zero data risk
- This task also resolves the task 0020 divergence issue flagged in 0101
- `accounts.balances` DEFAULT '[]' kept — non-optional field, default is INSERT convenience
- FK transactions→ledgers preserved as original `ON DELETE NO ACTION` (no behavior change)

## Deploy: reset DB before running new migrations

Migration file checksums changed — sqlx will reject them on any DB that ran the old versions.
Full schema wipe required (migrations use `CREATE TABLE` without `IF NOT EXISTS`).

Connect via SSM tunnel + psql:

```sql
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
```

Then redeploy the migration Lambda (CDK stack `Explorer-{env}-Migration`).
