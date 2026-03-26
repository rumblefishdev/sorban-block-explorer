---
type: research
status: mature
spawned_from: '0007'
---

# Drizzle ORM + PostgreSQL Range Partitioning

## Native Support: NO

Drizzle ORM does **not** support `PARTITION BY RANGE` (or any partitioning) in its schema DSL. No `partitionBy` option exists on `pgTable`. Verified as of drizzle-orm 0.45.1 / drizzle-kit 0.31.10 (March 2026). The canonical open issue: [drizzle-orm#2854](https://github.com/drizzle-team/drizzle-orm/issues/2854) (100+ upvotes, no ETA, last community comment July 2025 "Any updates?" without response).

## Querying Partitioned Tables: WORKS

Once partitioned tables exist in PostgreSQL, Drizzle's query builder (select, insert, update, delete) works transparently. PostgreSQL handles partition routing. No known issues.

## Workaround: Custom Migration SQL

### Approach A: Edit generated migration

1. Define table in Drizzle schema as normal `pgTable` (for type-safe queries)
2. `drizzle-kit generate` produces `.sql` migration
3. Manually edit the `CREATE TABLE` to add `PARTITION BY RANGE(column)`
4. Add `CREATE TABLE ... PARTITION OF ...` for child partitions

### Approach B: Custom migration file

```bash
drizzle-kit generate --custom --name=create-partitioned-table
```

Write raw SQL in the generated file. Keep a matching `pgTable` definition for type inference.

### Example

```typescript
// Schema (for type-safe queries only)
export const operations = pgTable('operations', {
  id: bigint('id', { mode: 'number' }).notNull(),
  transactionId: bigint('transaction_id', { mode: 'number' }).notNull(),
  // ... other columns
});
```

```sql
-- Migration (hand-written or edited)
CREATE TABLE operations (
  id bigint NOT NULL,
  transaction_id bigint NOT NULL
  -- ... other columns
) PARTITION BY RANGE (transaction_id);
```

## Foreign Keys + CASCADE on Partitioned Tables

### PostgreSQL rules

1. **PK on partitioned table** must include all partition key columns (PG requirement)
2. **FK FROM partitioned table** to a non-partitioned table works normally with `ON DELETE CASCADE`
3. **FK TO a partitioned table** requires the referenced unique constraint to include the partition key — this makes single-column FK references difficult

### Our schema — all FKs are FROM partitioned TO non-partitioned

All 4 partitioned tables reference non-partitioned parents. This is the easy case:

| Partitioned Table          | Partition Key    | FK Column        | References                 | Works?                                                          |
| -------------------------- | ---------------- | ---------------- | -------------------------- | --------------------------------------------------------------- |
| `operations`               | `transaction_id` | `transaction_id` | `transactions(id)`         | Yes — FK column = partition key, PK = `(id, transaction_id)`    |
| `soroban_invocations`      | `created_at`     | `transaction_id` | `transactions(id)`         | Yes — FK to non-partitioned table, PK must include `created_at` |
| `soroban_events`           | `created_at`     | `transaction_id` | `transactions(id)`         | Yes — FK to non-partitioned table, PK must include `created_at` |
| `liquidity_pool_snapshots` | `created_at`     | `pool_id`        | `liquidity_pools(pool_id)` | Yes — FK to non-partitioned table, PK must include `created_at` |

### PK design implication

Every partitioned table's PK must be composite, including the partition key:

- `operations`: `PRIMARY KEY (id, transaction_id)`
- `soroban_invocations`: `PRIMARY KEY (id, created_at)`
- `soroban_events`: `PRIMARY KEY (id, created_at)`
- `liquidity_pool_snapshots`: `PRIMARY KEY (id, created_at)`

This must be hand-written in migration SQL since Drizzle can't express partitioning.

Drizzle's `.references()` API passes through FK definitions correctly — no Drizzle-specific issues.

## Introspection Caveat

`drizzle-kit introspect` sees each partition as a separate table. PR #4355 aims to fix this by ignoring partition children.

## Sources

- [Feature Request #2854](https://github.com/drizzle-team/drizzle-orm/issues/2854)
- [Discussion #2093](https://github.com/drizzle-team/drizzle-orm/discussions/2093)
- [Custom Migrations docs](https://orm.drizzle.team/docs/kit-custom-migrations)
- [PostgreSQL Partitioning docs](https://www.postgresql.org/docs/current/ddl-partitioning.html)
