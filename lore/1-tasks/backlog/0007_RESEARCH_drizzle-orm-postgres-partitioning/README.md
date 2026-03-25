---
id: '0007'
title: 'Research: Drizzle ORM with PostgreSQL partitioning and advanced features'
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ['0008', '0009', '0010', '0011', '0012']
tags: [priority-high, effort-small, layer-research]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
---

# Research: Drizzle ORM with PostgreSQL partitioning and advanced features

## Summary

Investigate Drizzle ORM compatibility with PostgreSQL features required by the block explorer schema: range partitioning, foreign keys with ON DELETE CASCADE on partitioned tables, tsvector generated columns, GIN indexes on JSONB and tsvector, CHECK constraints, and Drizzle Kit migration generation for these features. This research must produce a compatibility matrix and documented workarounds for any gaps.

## Status: Backlog

## Context

The block explorer schema uses several advanced PostgreSQL features that are not universally supported by ORMs. Drizzle ORM is the chosen data access layer, and its compatibility with these features must be validated before implementation begins. The schema contains 4 partitioned tables, approximately 10 JSONB columns, GIN indexes, full-text search via tsvector, and CHECK constraints.

### Partitioned Tables

Four tables use PostgreSQL native range partitioning:

1. **operations** -- partitioned by `transaction_id` range. This keeps transaction children aligned with the ingestion write pattern and cascade cleanup behavior.
2. **soroban_invocations** -- partitioned monthly by `created_at`. Supports recent-history access patterns and time-sliced reads.
3. **soroban_events** -- partitioned monthly by `created_at`. Supports pattern matching, downstream interpretation jobs, and contract event timeline queries.
4. **liquidity_pool_snapshots** -- partitioned monthly by `created_at`. Supports time-series chart endpoint reads and interval queries.

### Partition Lifecycle

Partitions are created ahead of time and dropped operationally -- not by application code. The application should not be responsible for creating or dropping partitions at runtime. This is an operational concern managed by infrastructure or scheduled maintenance tasks.

### Foreign Keys and CASCADE on Partitioned Tables

Several partitioned tables have foreign key relationships with ON DELETE CASCADE:

- `operations.transaction_id` references `transactions(id) ON DELETE CASCADE`
- `soroban_invocations.transaction_id` references `transactions(id) ON DELETE CASCADE`
- `soroban_events.transaction_id` references `transactions(id) ON DELETE CASCADE`
- `liquidity_pool_snapshots.pool_id` references `liquidity_pools(pool_id) ON DELETE CASCADE`

PostgreSQL has specific requirements for foreign keys on partitioned tables (the FK columns must be part of the partition key in some configurations). The research must determine if Drizzle ORM can define these relationships correctly.

### tsvector Generated Column

The `soroban_contracts` table includes a `search_vector` column defined as:

```sql
search_vector TSVECTOR GENERATED ALWAYS AS (
    to_tsvector('english', coalesce(metadata->>'name', ''))
) STORED
```

This is a PostgreSQL-specific generated column using a function expression. The research must determine if Drizzle schema definitions can express this, or if it requires raw SQL in migrations.

### GIN Indexes

GIN indexes are used on multiple JSONB and tsvector columns:

- `operations.details` USING GIN -- supports queries into operation-specific JSONB fields
- `soroban_events.topics` USING GIN -- supports event signature and topic structure queries
- `soroban_contracts.search_vector` USING GIN -- supports full-text search

### CHECK Constraints

The `tokens` table uses a CHECK constraint:

```sql
asset_type VARCHAR(10) NOT NULL CHECK (asset_type IN ('classic', 'sac', 'soroban'))
```

### JSONB Columns

Approximately 10 JSONB columns exist across the schema:

- `operations.details`, `soroban_invocations.function_args`, `soroban_invocations.return_value`
- `soroban_events.topics`, `soroban_events.data`
- `soroban_contracts.metadata`, `tokens.metadata`, `nfts.metadata`
- `accounts.balances`, `liquidity_pools.asset_a`, `liquidity_pools.asset_b`, `liquidity_pools.reserves`
- `liquidity_pool_snapshots.reserves`, `event_interpretations.structured_data`

### Drizzle Kit Migration Generation

Drizzle Kit generates SQL migrations from schema definitions. The research must determine which of the above features can be expressed in Drizzle schema definitions (and thus auto-generate migrations) vs which require manual SQL migration additions.

## Research Questions

- Does Drizzle ORM support PostgreSQL range partitioning in schema definitions? Can `PARTITION BY RANGE` be expressed in the Drizzle schema, or must it be added via raw SQL migrations?
- Can Drizzle define foreign keys with ON DELETE CASCADE on partitioned tables? Are there limitations related to partition key requirements?
- Does Drizzle support `GENERATED ALWAYS AS ... STORED` columns, particularly with function expressions like `to_tsvector()`?
- Can GIN indexes be defined in Drizzle schema definitions, or must they be added via raw SQL?
- Does Drizzle support CHECK constraints in schema definitions?
- How does Drizzle Kit handle migration generation for features it cannot express natively? Can raw SQL be injected into generated migrations safely?
- What is the Drizzle ORM version compatibility matrix for these PostgreSQL features?
- Are there known issues or community workarounds for partitioned table support in Drizzle?
- How does Drizzle handle JSONB column typing in TypeScript? Can JSONB columns be strongly typed with generics?

## Acceptance Criteria

- [ ] Compatibility matrix: each PostgreSQL feature vs Drizzle ORM native support (yes/no/partial)
- [ ] Documented workaround for each unsupported feature (raw SQL migration, custom extension, etc.)
- [ ] Drizzle schema definition examples for partitioned tables (or confirmation that raw SQL is needed)
- [ ] Foreign key + CASCADE on partitioned tables: working approach documented
- [ ] tsvector generated column: working approach documented
- [ ] GIN index definition: working approach documented
- [ ] CHECK constraint: working approach documented
- [ ] JSONB column TypeScript typing approach documented
- [ ] Drizzle Kit migration workflow documented for mixed native + raw SQL features

## Notes

- The schema evolution rules state: avoid replacing explicit relational structure with oversized generic JSON blobs. Understanding how Drizzle handles JSONB typing is important for maintaining this discipline.
- Partition lifecycle (create ahead of time, drop operationally) is an infrastructure concern, not an ORM concern. But the ORM must at least not break when querying partitioned tables.
- The ingestion path uses per-ledger database transactions with batch insertion of child rows. Drizzle transaction support and batch insert performance are relevant but outside the scope of this specific research task.
