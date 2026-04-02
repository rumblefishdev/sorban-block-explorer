---
id: '0102'
title: 'Rewrite SQL migrations 0002-0006 to derive from domain types'
type: REFACTOR
status: backlog
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
---

# Rewrite SQL migrations 0002-0006 to derive from domain types

## Summary

Delete migrations 0002-0006 and rewrite them with domain types (`crates/domain/`) as source of truth. The DB is empty — no data risk. This establishes the correct workflow going forward: types define the contract, migrations implement it.

## Status: Backlog

**Current state:** Not started.

## Context

Migrations 0002-0006 were written before Rust domain types existed (the TS→Rust migration gap). Task 0101 created domain types by reverse-engineering the existing migrations. Now we invert the relationship: domain types are the spec, migrations are derived.

Known issues with current migrations:

- Task 0020 DDL spec diverges from actual migration 0006 (nfts PK structure, column sizes, nullability, fee_bps NOT NULL)
- No single source of truth — changes to schema require updating both domain types and migrations independently

### Target workflow (going forward)

1. Define/update struct in `crates/domain/`
2. Write migration SQL that matches the struct exactly
3. Both in the same PR

## Implementation Plan

### Step 1: Delete migrations 0002-0006

Move to `.trash/` per project policy. Keep 0001 (ledgers + transactions) — domain types `Ledger`/`Transaction` already existed before migrations.

### Step 2: Rewrite migration for operations

Derive DDL from `domain::operation::Operation`. Verify every field, type, nullability matches.

### Step 3: Rewrite migration for soroban_contracts

Derive from `domain::soroban::SorobanContract`. Include `search_vector` TSVECTOR generated column (DB-only, excluded from domain struct by design).

### Step 4: Rewrite migration for soroban_invocations + soroban_events

Derive from `domain::soroban::SorobanInvocation` and `domain::soroban::SorobanEvent`. Include partitioning and initial partitions.

### Step 5: Rewrite migration for accounts + tokens

Derive from `domain::account::Account` and `domain::token::Token`.

### Step 6: Rewrite migration for nfts + pools + snapshots

Derive from `domain::nft::Nft`, `domain::pool::LiquidityPool`, `domain::pool::LiquidityPoolSnapshot`. Include partitioning for snapshots.

### Step 7: Verify migrations apply cleanly

`sqlx migrate run` against a fresh PostgreSQL instance.

### Step 8: Close/update tasks 0018, 0019, 0020

Update status of DDL tasks to reflect the rewrite.

## Acceptance Criteria

- [ ] Migrations 0002-0006 rewritten — every column matches domain struct field
- [ ] Domain types are the documented source of truth for schema
- [ ] `sqlx migrate run` applies cleanly to fresh PostgreSQL
- [ ] Indexes, FKs, constraints, partitioning preserved
- [ ] DB-only columns (search_vector) documented as exceptions
- [ ] Tasks 0018, 0019, 0020 updated/closed

## Notes

- DB is empty — this is a pure rewrite with zero data risk
- Migration 0001 (ledgers + transactions) stays as-is — it predates the domain types gap
- This task also resolves the task 0020 divergence issue flagged in 0101
