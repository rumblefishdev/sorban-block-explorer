---
title: 'Drizzle ORM + PostgreSQL Advanced Features: Compatibility Matrix'
type: synthesis
status: mature
spawned_from: '0007'
spawns: []
tags: [drizzle, postgresql, compatibility]
links: []
history: []
---

# Drizzle ORM + PostgreSQL Advanced Features: Compatibility Matrix

## Compatibility Matrix

| PostgreSQL Feature                | Drizzle Schema DSL  | drizzle-kit generate  | Workaround                                                             |
| --------------------------------- | ------------------- | --------------------- | ---------------------------------------------------------------------- |
| `PARTITION BY RANGE`              | **No**              | **No**                | Edit migration SQL or `--custom`                                       |
| Child partitions (`PARTITION OF`) | **No**              | **No**                | Custom migration files                                                 |
| FK with `ON DELETE CASCADE`       | **Yes**             | **Yes**               | N/A                                                                    |
| FK FROM partitioned table         | **Yes**             | **Yes**               | N/A (all our FKs go FROM partitioned TO non-partitioned)               |
| FK TO partitioned table           | **Partial**         | **No**                | Hand-write composite PK in migration (PG requires partition key in PK) |
| Composite PK (with partition key) | **Yes**             | **Yes**               | Must define in schema — PG requires partition key in PK                |
| `GENERATED ALWAYS AS ... STORED`  | **Yes** (v0.32+)    | **Yes**               | N/A                                                                    |
| `tsvector` column type            | **No** (customType) | **Yes**               | `customType<{ data: string }>`                                         |
| GIN indexes                       | **Yes**             | **Yes** (kit >= 0.28) | N/A                                                                    |
| GIN with operator class           | **Yes**             | **Yes** (kit >= 0.28) | `.op('gin_trgm_ops')`                                                  |
| CHECK constraints                 | **Yes**             | **Partial**           | May need manual migration edit                                         |
| JSONB columns                     | **Yes**             | **Yes**               | N/A                                                                    |
| JSONB TypeScript typing           | **Yes**             | N/A                   | `.$type<T>()` (compile-time only)                                      |
| JSONB nested field queries        | **No**              | N/A                   | Raw `sql` operators                                                    |

## Version Compatibility Matrix

| Feature                          | Min drizzle-orm                     | Min drizzle-kit                                                                         | Min PostgreSQL |
| -------------------------------- | ----------------------------------- | --------------------------------------------------------------------------------------- | -------------- |
| `PARTITION BY RANGE`             | N/A (not supported)                 | N/A (custom SQL)                                                                        | 10             |
| FK FROM partitioned tables       | any                                 | N/A (custom SQL)                                                                        | 11             |
| FK TO partitioned tables         | any                                 | N/A (custom SQL)                                                                        | 12             |
| `GENERATED ALWAYS AS ... STORED` | 0.32.0                              | 0.32.0                                                                                  | 12             |
| `tsvector` (customType)          | any                                 | any                                                                                     | 9.6            |
| GIN indexes                      | any                                 | 0.28.6 (full index field support)                                                       | 9.6            |
| GIN with operator class          | any                                 | 0.28.6                                                                                  | 9.6            |
| CHECK constraints                | ~0.30+ (exact version undocumented) | 0.28.0 (partial — bug [#3520](https://github.com/drizzle-team/drizzle-orm/issues/3520)) | any            |
| JSONB + `.$type<T>()`            | any                                 | any                                                                                     | 9.4            |

**Current latest versions (March 2026):** drizzle-orm 0.45.1, drizzle-kit 0.31.10 (+ v1.0.0-beta.19 in beta channel).

**Our minimum versions:** drizzle-orm >= 0.32.0, drizzle-kit >= 0.28.6, PostgreSQL >= 11 (>= 12 only if FKs referencing partitioned parents are needed).

**Note on v1.0.0-beta:** In beta.12+, `generatedAlwaysAs()` no longer accepts raw string literals — only `sql` template expressions and callbacks. Our examples use `sql` templates, so they are compatible with both stable and beta.

## Recommendation for Block Explorer Schema

### Fully native (no workaround needed)

- GIN indexes on JSONB and tsvector columns
- tsvector generated columns (with `customType`)
- JSONB columns with TypeScript typing
- Foreign keys with ON DELETE CASCADE (on non-partitioned tables)
- Standard indexes, unique constraints, NOT NULL

### Needs manual migration edits

- **4 partitioned tables** — define in schema for queries, add `PARTITION BY RANGE` in migration SQL
- **Composite PKs** — all 4 partitioned tables need PK including partition key (e.g., `PRIMARY KEY (id, created_at)`)
- **CHECK constraint on tokens.asset_type** — define in schema, verify drizzle-kit emits it (if not, add manually)

### Recommended Migration Strategy

```
drizzle-kit generate          # Base DDL from schema
  |-- edit SQL                # Add PARTITION BY RANGE to 4 tables
  |-- edit SQL                # Fix composite PKs for partitioned tables
drizzle-kit generate --custom # Child partition DDL
drizzle-kit generate --custom # Monthly partition management
migrate()                     # Apply at deploy
```

### Version Requirements

- drizzle-orm >= 0.32.0 (generated columns)
- drizzle-kit >= 0.28.6 (full index field support including GIN operator classes)
- PostgreSQL >= 11 (FKs from partitioned tables; PostgreSQL 12+ only needed for FKs to partitioned parents)

## Impact on Implementation Tasks

- **0015 (Drizzle config)** — use `generate` + `migrate` workflow, not `push`
- **0016 (ledgers/transactions schema)** — standard tables, fully native
- **0017 (operations schema)** — partitioned by `transaction_id`, needs custom migration
- **0018 (soroban tables)** — `invocations` and `events` partitioned monthly, `contracts` has tsvector + GIN (native)
- **0019 (tokens/accounts)** — CHECK constraint on `asset_type`, JSONB `balances`
- **0020 (NFTs/pools/snapshots)** — `pool_snapshots` partitioned monthly
- **0021 (migration framework)** — must support mixed native + custom SQL workflow
- **0022 (partition management)** — operational concern, use `--custom` migrations or external tooling
