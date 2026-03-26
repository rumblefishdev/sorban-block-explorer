---
type: research
status: mature
spawned_from: '0007'
---

# Drizzle Kit Migration Workflow

## Two Approaches

|                 | `generate` + `migrate` | `push`          |
| --------------- | ---------------------- | --------------- |
| Migration files | Yes (SQL)              | No              |
| Production use  | Recommended            | Not recommended |
| Custom SQL      | Yes (edit files)       | Not possible    |
| Reproducible    | Yes                    | No              |

**For our project: use `generate` + `migrate`** (partitioned tables require custom SQL).

## Standard Workflow

1. Define schema in TypeScript
2. `drizzle-kit generate` — diffs TS schema vs latest snapshot, produces `.sql` + snapshot JSON
3. `drizzle-kit migrate` (or `migrate()` at runtime) — applies pending migrations in order
4. Migrations are **append-only** — new files only, never overwrites old ones

## Custom SQL Injection

### Edit generated migration files

- Safe — drizzle-kit **never overwrites** existing migration files
- Diffs against snapshot in `drizzle/meta/`, not SQL files
- **Best practice:** Only add DDL Drizzle doesn't track (partitions, triggers, functions)
- **Don't** manually add/remove columns — let Drizzle manage those

### Custom migration files

```bash
drizzle-kit generate --custom --name=create-monthly-partitions
```

Creates empty `.sql` file for hand-written SQL. Runs in order with other migrations.

### `sql` template in schema

Limited to expressions within Drizzle-managed constructs:

- Default values: `default(sql\`...\`)`
- Check constraints: `check(sql\`...\`)`
- Index expressions: `index().on(sql\`...\`)`
- Generated columns: `.generatedAlwaysAs(sql\`...\`)`

**Cannot** define structural DDL like `PARTITION BY RANGE` in schema.

## Recommended Workflow for Our Schema

1. Define tables in Drizzle schema as normal `pgTable` (for type inference + queries)
2. `drizzle-kit generate` for base migration
3. Edit generated SQL: add `PARTITION BY RANGE(...)` to 4 partitioned tables
4. `drizzle-kit generate --custom` for partition DDL (child partitions)
5. For ongoing partition management (new monthly partitions): custom migrations
6. Apply with `migrate()` at deploy time

## Key Rules

- **Never use `push` in production** — no custom SQL, no audit trail
- **Commit migration files AND `drizzle/meta/` snapshots** to git
- **Review generated SQL** before applying — differ can produce unexpected DROPs
- **Keep Drizzle-managed and hand-managed DDL separate**
- Use `drizzle-kit check` to validate migration consistency

## Recent Drizzle Kit Features (v0.30+)

- CHECK constraints in schema
- Views (regular + materialized)
- Identity columns (GENERATED ALWAYS AS IDENTITY)
- PostgreSQL schemas/namespaces
- Row-Level Security policies
- Sequences
- `--custom` flag for blank migrations

**Still not supported:** partitioning, triggers, stored procedures, custom operators.

## Sources

- [Drizzle Migrations docs](https://orm.drizzle.team/docs/migrations)
- [Drizzle Custom Migrations](https://orm.drizzle.team/docs/kit-custom-migrations)
