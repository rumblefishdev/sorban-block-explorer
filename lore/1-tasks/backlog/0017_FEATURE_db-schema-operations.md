---
id: '0017'
title: 'DB schema: operations table with transaction_id partitioning'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0016', '0009']
tags: [priority-high, effort-small, layer-database]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# DB schema: operations table with transaction_id partitioning

## Summary

Implement the Drizzle ORM schema definition and SQL DDL for the `operations` table. This table stores per-operation structure for transaction analysis and is partitioned by `transaction_id` range -- notably NOT time-based partitioning.

## Status: Backlog

**Current state:** Not started.

## Context

Operations are child records of transactions. Each transaction may contain one or more operations of varying types. The `details` column is JSONB because operation-specific fields vary heavily by operation type (e.g., CreateAccount vs InvokeHostFunction have completely different payloads).

### Full DDL

```sql
CREATE TABLE operations (
    id              BIGSERIAL PRIMARY KEY,
    transaction_id  BIGINT REFERENCES transactions(id) ON DELETE CASCADE,
    type            VARCHAR(50) NOT NULL,
    details         JSONB NOT NULL,
    INDEX idx_tx (transaction_id),
    INDEX idx_details (details) USING GIN
) PARTITION BY RANGE (transaction_id);
```

### Design Notes

- **PARTITION BY RANGE (transaction_id)** -- This is range-based partitioning on the transaction surrogate ID, NOT time-based. This keeps transaction children co-located with their parent's ID range, which aligns with the ingestion write pattern and cascade cleanup behavior.
- **ON DELETE CASCADE** from transactions -- deleting a transaction automatically removes all its operations. This maintains referential integrity without requiring application-level cleanup.
- **GIN index on details** -- the `details` JSONB column has a GIN index to support queries against the variable-shaped operation payloads.

### INVOKE_HOST_FUNCTION details JSONB Structure

For Soroban `invoke_host_function` operations, the `details` JSONB column contains a decoded structure:

```json
{
  "contractId": "string",
  "functionName": "string",
  "functionArgs": ["unknown[]  -- decoded ScVal values"],
  "returnValue": "unknown  -- decoded ScVal value"
}
```

- `contractId`: the Soroban contract address being invoked
- `functionName`: the name of the contract function called
- `functionArgs`: array of decoded ScVal arguments (shape varies per contract function)
- `returnValue`: the decoded ScVal return value (shape varies per contract function)

Other operation types will have different `details` shapes corresponding to their specific fields.

### Partition Strategy

- Partitions are created based on `transaction_id` ranges, NOT monthly time windows.
- Partition boundaries are determined by monitoring transaction ID growth.
- Partitions must be created ahead of time -- application code MUST NOT create or drop partitions ad hoc.
- See task 0022 for partition management automation details.

## Implementation Plan

### Step 1: Drizzle schema definition

Define the operations table using Drizzle ORM schema builder. Include all columns, the BIGSERIAL primary key, FK to transactions(id) with ON DELETE CASCADE, and both indexes.

### Step 2: Partition configuration

Configure the table as PARTITION BY RANGE (transaction_id) in the DDL. Create initial partition(s) for the expected starting transaction_id range.

### Step 3: Generate migration

Use Drizzle Kit to generate the migration. Verify the generated DDL includes the partitioning clause and CASCADE behavior.

### Step 4: Validate cascade behavior

Test that deleting a transaction row cascades to remove its operation rows.

### Step 5: Validate GIN index

Test that the GIN index on details supports containment queries (@>) against the JSONB column.

## Acceptance Criteria

- [ ] Drizzle schema for operations table matches the DDL specification
- [ ] Table is defined with PARTITION BY RANGE (transaction_id)
- [ ] FK to transactions(id) with ON DELETE CASCADE is enforced
- [ ] GIN index on details column is created
- [ ] Index on transaction_id is created
- [ ] At least one initial partition exists for the starting transaction_id range
- [ ] Cascade delete from transactions to operations works correctly
- [ ] JSONB containment queries work against the GIN index
- [ ] Migration applies cleanly to a fresh PostgreSQL instance

## Notes

- Drizzle ORM has limited native support for PostgreSQL partitioned tables. The migration may need raw SQL supplements for the PARTITION BY clause and partition creation.
- Partition management automation is covered in task 0022. This task only needs to create the partitioned table definition and an initial partition.
- The transaction_id-based partitioning is intentionally different from the time-based partitioning used by soroban_invocations, soroban_events, and liquidity_pool_snapshots.
