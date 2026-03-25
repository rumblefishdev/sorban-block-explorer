---
id: '0020'
title: 'DB schema: NFTs, liquidity pools, and pool snapshots tables'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0012']
tags: [priority-medium, effort-medium, layer-database]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# DB schema: NFTs, liquidity pools, and pool snapshots tables

## Summary

Implement the Drizzle ORM schema definitions and SQL DDL for three tables: `nfts`, `liquidity_pools`, and `liquidity_pool_snapshots`. These represent derived explorer entities for NFT display, pool state, and time-series pool analytics.

## Status: Backlog

**Current state:** Not started.

## Context

NFTs and liquidity pools are derived-state entities built on indexed chain data. Pool snapshots are an append-only time-series table for chart endpoints. Together, these tables support the explorer's NFT gallery, liquidity pool detail pages, and pool analytics charts.

### Full DDL

#### nfts

```sql
CREATE TABLE nfts (
    id                BIGSERIAL PRIMARY KEY,
    contract_id       VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    token_id          VARCHAR(128) NOT NULL,
    collection_name   VARCHAR(100),
    owner_account     VARCHAR(56),
    name              VARCHAR(100),
    media_url         TEXT,
    metadata          JSONB,
    minted_at_ledger  BIGINT REFERENCES ledgers(sequence),
    last_seen_ledger  BIGINT REFERENCES ledgers(sequence),
    UNIQUE (contract_id, token_id),
    INDEX idx_contract (contract_id),
    INDEX idx_owner (owner_account)
);
```

#### liquidity_pools

```sql
CREATE TABLE liquidity_pools (
    pool_id             VARCHAR(64) PRIMARY KEY,
    asset_a             JSONB NOT NULL,
    asset_b             JSONB NOT NULL,
    fee_bps             INT,
    reserves            JSONB NOT NULL,
    total_shares        NUMERIC(28, 7),
    tvl                 NUMERIC(28, 7),
    created_at_ledger   BIGINT REFERENCES ledgers(sequence),
    last_updated_ledger BIGINT REFERENCES ledgers(sequence),
    INDEX idx_last_updated (last_updated_ledger DESC)
);
```

#### liquidity_pool_snapshots

```sql
CREATE TABLE liquidity_pool_snapshots (
    id               BIGSERIAL PRIMARY KEY,
    pool_id          VARCHAR(64) REFERENCES liquidity_pools(pool_id) ON DELETE CASCADE,
    ledger_sequence  BIGINT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    reserves         JSONB NOT NULL,
    total_shares     NUMERIC(28, 7),
    tvl              NUMERIC(28, 7),
    volume           NUMERIC(28, 7),
    fee_revenue      NUMERIC(28, 7),
    INDEX idx_pool_time (pool_id, created_at DESC)
) PARTITION BY RANGE (created_at);
```

### Design Notes

#### nfts

- **contract_id** FK to `soroban_contracts(contract_id)` -- links each NFT to its contract entity.
- **minted_at_ledger** and **last_seen_ledger** both FK to `ledgers(sequence)`. These track the ledger at which the NFT was first minted and most recently observed.
- **UNIQUE(contract_id, token_id)** -- token_id uniqueness is scoped by contract. Two different contracts may use the same token_id value.
- **metadata** and **media_url** remain optional because NFT contract conventions and metadata quality vary heavily across the ecosystem.
- **Transfer history** is derived from stored soroban_events and linked transactions, NOT a separate canonical NFT transfer table. The explorer reconstructs transfer history from event data.
- NFTs use the same watermark-guarded upsert pattern as tokens and accounts.

#### liquidity_pools

- **asset_a** and **asset_b** are JSONB NOT NULL because pool assets may span classic Stellar assets and Soroban-native token identities. The JSONB structure accommodates both.
- **reserves** is JSONB NOT NULL for the same reason -- reserve amounts reference the mixed-type asset pair.
- **total_shares** and **tvl** use NUMERIC(28, 7) for Stellar-precision decimal amounts.
- **created_at_ledger** and **last_updated_ledger** FK to ledgers(sequence).
- **idx_last_updated** (last_updated_ledger DESC) supports queries for recently active pools.
- Pools are **upserted with ledger-sequence watermarks** -- same pattern as tokens and accounts. Older backfill MUST NOT overwrite newer live state.
- Pool transaction history is derived from transactions, operations, and Soroban events rather than a dedicated pool-transactions table.

#### liquidity_pool_snapshots

- **FK CASCADE from pools**: `pool_id REFERENCES liquidity_pools(pool_id) ON DELETE CASCADE`. Deleting a pool removes all its snapshots.
- **NUMERIC(28, 7)** for total_shares, tvl, volume, and fee_revenue -- consistent precision for all financial metrics.
- **Monthly partitioned** by `created_at` using PARTITION BY RANGE.
- **APPEND-ONLY**: snapshot rows are written in ledger order and are NEVER updated after insertion. They represent point-in-time state for chart endpoints.
- **volume** and **fee_revenue** are explorer-derived metrics, not chain primitives. They are computed from transaction/event data during ingestion.
- **idx_pool_time** (pool_id, created_at DESC) supports chart queries that retrieve recent snapshots for a specific pool.

## Implementation Plan

### Step 1: Drizzle schema for nfts

Define the table with all columns, FK to soroban_contracts, two FKs to ledgers, UNIQUE constraint on (contract_id, token_id), and both indexes.

### Step 2: Drizzle schema for liquidity_pools

Define the table with all columns, two FKs to ledgers, JSONB columns for assets and reserves, NUMERIC columns, and the last_updated_ledger DESC index.

### Step 3: Drizzle schema for liquidity_pool_snapshots

Define the partitioned table with FK CASCADE to liquidity_pools, all NUMERIC columns, JSONB reserves, and the composite index. Configure PARTITION BY RANGE (created_at).

### Step 4: Generate migrations

Use Drizzle Kit to generate migration files. Supplement with raw SQL for partitioning if needed.

### Step 5: Create initial monthly partitions for snapshots

Create initial partitions for liquidity_pool_snapshots covering at least the next 3 months. Naming convention: `liquidity_pool_snapshots_y{YYYY}m{MM}`.

### Step 6: Validate cascade behavior

Test that deleting a liquidity pool cascades to remove all its snapshots.

### Step 7: Validate UNIQUE constraint on nfts

Test that (contract_id, token_id) uniqueness is enforced -- duplicate pairs are rejected, but the same token_id under different contracts is allowed.

## Acceptance Criteria

- [ ] Drizzle schema for nfts matches DDL with all FKs and UNIQUE constraint
- [ ] Drizzle schema for liquidity_pools matches DDL with JSONB and NUMERIC columns
- [ ] Drizzle schema for liquidity_pool_snapshots matches DDL with monthly partitioning
- [ ] FK CASCADE from snapshots to pools works correctly
- [ ] UNIQUE(contract_id, token_id) is enforced on nfts
- [ ] All indexes are created correctly
- [ ] Initial monthly partitions exist for liquidity_pool_snapshots
- [ ] NUMERIC(28, 7) precision is verified for financial columns
- [ ] Migration applies cleanly to a fresh PostgreSQL instance

## Notes

- liquidity_pool_snapshots is one of four partitioned tables in the schema. Partition management automation is covered in task 0022.
- The append-only nature of snapshots means there is no upsert logic for this table -- only INSERTs.
- NFT and pool upserts share the watermark pattern with tokens and accounts (task 0019). Consider implementing the watermark utility once and reusing it.
