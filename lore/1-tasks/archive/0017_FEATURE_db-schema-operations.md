---
id: '0017'
title: 'DB schema: operations table with transaction_id partitioning'
type: FEATURE
status: completed
related_adr: ['0005']
related_tasks: ['0016', '0009', '0092']
tags: [priority-high, effort-small, layer-database]
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
  - date: 2026-03-31
    status: active
    who: stkrolikiewicz
    note: 'Updated: plain SQL migration instead of Drizzle (per research 0092)'
  - date: 2026-03-31
    status: completed
    who: stkrolikiewicz
    note: >
      Migration created and verified on PostgreSQL via docker-compose.
      1 SQL file, 6 columns, 3 indexes, 1 partition.
      All 13 acceptance criteria verified (partitioning, cascade, GIN, source_account).
      Two columns added vs original spec: application_order, source_account.
---

# DB schema: operations table with transaction_id partitioning

## Summary

Plain SQL migration for the `operations` table. Partitioned by `transaction_id` range. Includes cascade delete from transactions, GIN index on JSONB details, and indexes for account-centric queries.

## Acceptance Criteria

- [x] SQL migration creates operations table matching the DDL specification
- [x] Table is defined with PARTITION BY RANGE (transaction_id)
- [x] Primary key includes partition key: (id, transaction_id)
- [x] `application_order` SMALLINT column exists (operation index within tx)
- [x] `source_account` VARCHAR(56) column exists with index
- [x] FK to transactions(id) with ON DELETE CASCADE is enforced
- [x] GIN index on details column is created
- [x] Index on transaction_id is created
- [x] Index on source_account is created
- [x] At least one initial partition exists (operations_p0)
- [x] Cascade delete from transactions to operations works correctly
- [x] JSONB containment queries (`@>`) work against the GIN index
- [x] Migration applies cleanly to a fresh PostgreSQL instance (docker-compose)

## Implementation Notes

**File:** `libs/database/drizzle/0001_create_operations.sql`

Plain SQL (not Drizzle Kit). Run via `psql`. Task 0094 will move to `crates/db/migrations/`.

```sql
CREATE TABLE operations (
    id                  BIGSERIAL,
    transaction_id      BIGINT NOT NULL,
    application_order   SMALLINT NOT NULL,
    source_account      VARCHAR(56) NOT NULL,
    type                VARCHAR(50) NOT NULL,
    details             JSONB NOT NULL,
    PRIMARY KEY (id, transaction_id),
    FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE
) PARTITION BY RANGE (transaction_id);
```

Indexes: `idx_operations_tx`, `idx_operations_source`, `idx_operations_details` (GIN).
Initial partition: `operations_p0` for transaction IDs 0–10M.

## Design Decisions

### From Plan

1. **PARTITION BY RANGE (transaction_id)** — range partitioning on surrogate ID, not time-based. Keeps transaction children co-located.

2. **ON DELETE CASCADE** — deleting a transaction removes its operations automatically.

3. **GIN index on details** — supports JSONB containment queries (`@>`) for variable-shaped operation payloads.

4. **Composite PK (id, transaction_id)** — PostgreSQL requires partition key in primary key for partitioned tables.

### Emerged

5. **Plain SQL instead of Drizzle** — original task specified Drizzle ORM schema. Changed per research 0092 (sqlx migrations, drop Drizzle Kit). Drizzle Kit doesn't support `PARTITION BY` syntax anyway.

6. **Added `application_order SMALLINT`** — not in original spec. Required to reconstruct operation ordering within a transaction (e.g., "operation 2 of 5"). Block explorer needs this for UI.

7. **Added `source_account VARCHAR(56)` + index** — not in original spec. Operations can have per-op source account (different from tx source). Without this column, "show operations by account X" requires JOIN + JSONB scan. Critical for block explorer account pages.

8. **Initial partition range 0–10M** — arbitrary starting point. Task 0022 handles partition automation based on transaction ID growth.

## Issues Encountered

- **Drizzle schema DSL lacks PARTITION BY** — Drizzle Kit can run raw SQL files, but the `pgTable()` DSL has no partitioning support. Since we're transitioning to sqlx (research 0092), we write plain SQL and run via `psql` rather than using Drizzle Kit's migration runner.

- **No Drizzle journal update** — the `drizzle/meta/_journal.json` is not updated for this migration since we don't use Drizzle Kit to run it. Not a problem since migration tooling is transitioning to sqlx.

## Future Work

- Task 0022 handles partition management automation (creating new partitions as transaction IDs grow).
- Task 0093 Step 4 updates tasks 0018-0021 to use plain SQL instead of Drizzle.
- Task 0094 moves all migrations to `crates/db/migrations/` with sqlx naming.
