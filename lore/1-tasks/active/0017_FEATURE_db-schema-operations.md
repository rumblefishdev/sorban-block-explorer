---
id: '0017'
title: 'DB schema: operations table with transaction_id partitioning'
type: FEATURE
status: active
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
    note: 'Updated: plain SQL migration instead of Drizzle (per research 0092 — sqlx migrations, drop Drizzle Kit)'
---

# DB schema: operations table with transaction_id partitioning

## Summary

Create the SQL migration for the `operations` table. This table stores per-operation structure for transaction analysis and is partitioned by `transaction_id` range.

> **Migration approach changed:** Research 0092 decided on sqlx migrations (plain SQL). This task creates a plain SQL migration file alongside the existing Drizzle migration from task 0016. Task 0094 will move all migrations to `crates/db/migrations/`.

## Status: Active

**Current state:** Ready to implement.

## Context

Operations are child records of transactions. Each transaction may contain one or more operations of varying types. The `details` column is JSONB because operation-specific fields vary heavily by operation type (e.g., CreateAccount vs InvokeHostFunction have completely different payloads).

### Full DDL

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

CREATE INDEX idx_operations_tx ON operations (transaction_id);
CREATE INDEX idx_operations_source ON operations (source_account);
CREATE INDEX idx_operations_details ON operations USING GIN (details);

-- Initial partition for transaction IDs 0 to 10,000,000
CREATE TABLE operations_p0 PARTITION OF operations
    FOR VALUES FROM (0) TO (10000000);
```

Note: Primary key must include the partition key (`transaction_id`) — PostgreSQL requirement for partitioned tables.

**Columns added vs original spec (research-informed):**

- `application_order SMALLINT` — operation index within transaction (0, 1, 2...). Required to reconstruct operation ordering in explorer UI.
- `source_account VARCHAR(56)` — operation-level source account (defaults to tx source, can be overridden per-op). Enables "show operations by account X" without JOIN + JSONB scan.
- `idx_operations_source` — index for account-centric queries.

### Design Notes

- **PARTITION BY RANGE (transaction_id)** — range-based partitioning on transaction surrogate ID, NOT time-based. Keeps transaction children co-located with their parent's ID range.
- **ON DELETE CASCADE** from transactions — deleting a transaction automatically removes all its operations.
- **GIN index on details** — supports JSONB containment queries (`@>`) against variable-shaped operation payloads.
- **Composite primary key** — `(id, transaction_id)` required because PostgreSQL partitioned tables need the partition key in the primary key.

### INVOKE_HOST_FUNCTION details JSONB Structure

For Soroban `invoke_host_function` operations, the `details` JSONB column contains:

```json
{
  "contractId": "string",
  "functionName": "string",
  "functionArgs": ["decoded ScVal values"],
  "returnValue": "decoded ScVal value"
}
```

Other operation types have different `details` shapes corresponding to their specific fields.

### Partition Strategy

- Partitions created based on `transaction_id` ranges, NOT monthly time windows.
- Partition boundaries determined by monitoring transaction ID growth.
- Partitions must be created ahead of time — application code MUST NOT create or drop partitions ad hoc.
- See task 0022 for partition management automation details.

## Implementation Plan

### Step 1: Write SQL migration

Create `libs/database/drizzle/0001_create_operations.sql` with plain SQL DDL (`;` separated, NOT Drizzle `--> statement-breakpoint` format). This sits alongside the existing 0016 migration. Task 0094 will later move all migrations to `crates/db/migrations/` with sqlx naming convention.

**Run via `psql`, not Drizzle Kit** — Drizzle Kit does not support `PARTITION BY` syntax.

### Step 2: Validate on fresh PostgreSQL

```bash
docker compose up -d postgres
# Apply 0016 first (ledgers + transactions)
psql $DATABASE_URL -f libs/database/drizzle/0000_create_ledgers_transactions.sql
# Apply 0017 (operations)
psql $DATABASE_URL -f libs/database/drizzle/0001_create_operations.sql
```

### Step 3: Validate partitioning

Insert rows within partition range, verify PG routes to `operations_p0`. Insert outside range — should error (no partition exists).

### Step 4: Validate cascade behavior

Insert a transaction + operations. Delete the transaction. Verify operations are gone.

### Step 5: Validate GIN index

Test JSONB containment queries (`@>`) against the `details` column.

## Acceptance Criteria

- [ ] SQL migration creates operations table matching the DDL specification
- [ ] Table is defined with PARTITION BY RANGE (transaction_id)
- [ ] Primary key includes partition key: (id, transaction_id)
- [ ] `application_order` SMALLINT column exists (operation index within tx)
- [ ] `source_account` VARCHAR(56) column exists with index
- [ ] FK to transactions(id) with ON DELETE CASCADE is enforced
- [ ] GIN index on details column is created
- [ ] Index on transaction_id is created
- [ ] Index on source_account is created
- [ ] At least one initial partition exists (operations_p0)
- [ ] Cascade delete from transactions to operations works correctly
- [ ] JSONB containment queries (`@>`) work against the GIN index
- [ ] Migration applies cleanly to a fresh PostgreSQL instance (docker-compose)

## Notes

- **No Drizzle ORM schema** — per research 0092, we use plain SQL migrations. No `.ts` schema file needed.
- **Run via `psql`** — Drizzle Kit does not support `PARTITION BY`. Apply manually or via script.
- Migration file goes to `libs/database/drizzle/` for now (alongside 0016). Task 0094 migrates everything to `crates/db/migrations/`.
- Partition management automation is covered in task 0022.
- The transaction_id-based partitioning is intentionally different from time-based partitioning planned for other tables.
- Initial partition range 0-10M is a starting point. More partitions added by task 0022 based on transaction ID growth.
- FK ON DELETE CASCADE on partitioned tables works in PG 12+. Constraint propagates automatically to new partitions.
- **Tasks 0018-0021 also reference Drizzle** — they will be updated in task 0093 (backlog cleanup). This task only updates 0017.
