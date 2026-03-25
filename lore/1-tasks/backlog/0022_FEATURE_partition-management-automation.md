---
id: '0022'
title: 'Partition management automation'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0017', '0018', '0020']
tags: [priority-medium, effort-medium, layer-database]
links:
  - docs/architecture/database-schema/database-schema-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Partition management automation

## Summary

Implement automated partition creation and management for all four partitioned tables in the block explorer schema. Three tables use monthly time-based partitioning and one uses transaction_id range-based partitioning. Partitions must be created ahead of time; application code must never create or drop partitions ad hoc.

## Status: Backlog

**Current state:** Not started.

## Context

The block explorer schema includes four partitioned tables, each with a different partitioning strategy. Partitions must exist before data arrives -- inserting into a partitioned table without a matching partition causes an error. This task ensures partitions are created proactively and managed through automation rather than manual intervention.

### Partitioned Tables

| Table                    | Partition Key            | Strategy                | Naming Convention                       |
| ------------------------ | ------------------------ | ----------------------- | --------------------------------------- |
| operations               | transaction_id (BIGINT)  | RANGE by transaction_id | Based on ID ranges                      |
| soroban_invocations      | created_at (TIMESTAMPTZ) | RANGE by month          | `soroban_invocations_y{YYYY}m{MM}`      |
| soroban_events           | created_at (TIMESTAMPTZ) | RANGE by month          | `soroban_events_y{YYYY}m{MM}`           |
| liquidity_pool_snapshots | created_at (TIMESTAMPTZ) | RANGE by month          | `liquidity_pool_snapshots_y{YYYY}m{MM}` |

### Time-Based Partitions (3 tables)

**soroban_invocations**, **soroban_events**, and **liquidity_pool_snapshots** are all partitioned by `RANGE (created_at)` with monthly boundaries.

Requirements:

- Create monthly partitions ahead of time -- minimum 3 months into the future at all times.
- Partition naming: `{table}_y{YYYY}m{MM}` (e.g., `soroban_events_y2026m04`, `soroban_events_y2026m05`, `soroban_events_y2026m06`).
- Each partition covers one calendar month: `FROM ('YYYY-MM-01') TO ('YYYY-{MM+1}-01')`.
- Schedule creation via EventBridge rule or CDK custom resource that runs periodically (e.g., weekly or monthly) to ensure future partitions always exist.

Example DDL for a monthly partition:

```sql
CREATE TABLE soroban_events_y2026m04 PARTITION OF soroban_events
    FOR VALUES FROM ('2026-04-01') TO ('2026-05-01');

CREATE TABLE soroban_invocations_y2026m04 PARTITION OF soroban_invocations
    FOR VALUES FROM ('2026-04-01') TO ('2026-05-01');

CREATE TABLE liquidity_pool_snapshots_y2026m04 PARTITION OF liquidity_pool_snapshots
    FOR VALUES FROM ('2026-04-01') TO ('2026-05-01');
```

### Transaction-ID-Based Partitions (operations)

The **operations** table is partitioned by `RANGE (transaction_id)`, which is fundamentally different from the time-based tables.

Requirements:

- Create partitions based on transaction_id ranges, not calendar months.
- Monitor transaction_id growth to determine when new partitions are needed.
- Range boundaries should be chosen based on expected transaction volume (e.g., partitions of 10 million transaction IDs each, adjusted based on observed growth).
- New partitions should be created before the current range is exhausted.

Example DDL for a range partition:

```sql
CREATE TABLE operations_range_0_10m PARTITION OF operations
    FOR VALUES FROM (0) TO (10000000);

CREATE TABLE operations_range_10m_20m PARTITION OF operations
    FOR VALUES FROM (10000000) TO (20000000);
```

### Retention Policy

- **Ledger and transaction history** is kept indefinitely. These are unpartitioned tables.
- **Partitioned tables** may be pruned ONLY if storage constraints require it. Pruning is an operational decision, NOT an automated process.
- Partition dropping is a manual operational action performed by an operator after review.
- The automation creates partitions ahead of time; it does NOT automatically drop old partitions.

### Key Principle

**Partitions are created ahead of time and dropped operationally. Application code MUST NOT create or drop partitions ad hoc.** The partition management automation is infrastructure-level tooling, not application-level logic embedded in the API or indexer.

## Implementation Plan

### Step 1: Partition creation Lambda or script

Create a Lambda function (or database script) that:

- Checks which partitions currently exist for each time-based table.
- Creates any missing partitions for the next 3+ months.
- Is idempotent -- running it multiple times does not create duplicate partitions or error on already-existing ones.

### Step 2: EventBridge schedule for time-based partitions

Set up an EventBridge rule that triggers the partition creation Lambda on a regular schedule (e.g., weekly or on the 1st of each month). This ensures future partitions are always created well in advance.

### Step 3: Transaction-ID partition monitoring

Implement monitoring for the operations table:

- Track the current maximum transaction_id in the database.
- Alert when the current partition range is approaching its upper bound (e.g., 80% consumed).
- Provide a mechanism (manual trigger or semi-automated) to create the next transaction_id range partition.

### Step 4: Initial partition seeding

Create the initial set of partitions as part of the database setup:

- Time-based tables: partitions covering the current month plus at least 3 future months.
- Operations table: initial partition covering the starting transaction_id range.

### Step 5: CDK integration

Wire the partition creation Lambda into the CDK stack:

- Deploy the Lambda as part of the infrastructure.
- Configure the EventBridge schedule rule.
- Optionally run the partition creation as a CDK custom resource during initial deployment.

### Step 6: Monitoring and alerting

Add CloudWatch alarms for:

- Time-based tables: alert if fewer than 2 future monthly partitions exist.
- Operations table: alert if the current partition range is above 80% consumed.
- Failed partition creation Lambda invocations.

## Acceptance Criteria

- [ ] Partition creation Lambda exists and creates monthly partitions for all 3 time-based tables
- [ ] Lambda creates partitions at least 3 months into the future
- [ ] Lambda is idempotent -- safe to run multiple times
- [ ] EventBridge schedule triggers the Lambda on a regular cadence
- [ ] Partition naming follows the convention: `{table}_y{YYYY}m{MM}`
- [ ] Initial partitions are seeded for all 4 partitioned tables
- [ ] Monitoring alerts when time-based tables have fewer than 2 future partitions
- [ ] Monitoring alerts when operations partition range is approaching exhaustion
- [ ] No application code in apps/api or apps/indexer creates or drops partitions
- [ ] Documentation exists for the manual partition pruning process
- [ ] Core history tables (ledgers, transactions) are never subject to automated pruning
- [ ] Documentation exists for the manual partition pruning process including operator review checklist

## Notes

- This task depends on the partitioned table definitions from tasks 0017 (operations), 0018 (soroban_invocations, soroban_events), and 0020 (liquidity_pool_snapshots).
- The partition creation Lambda should use IF NOT EXISTS semantics or equivalent checks to remain idempotent.
- Transaction-ID-based partition management is inherently less predictable than time-based. The initial range size should be chosen conservatively and adjusted based on observed indexing volume.
- Partition pruning is deliberately NOT automated. Any partition drop must be reviewed by an operator considering storage pressure, data retention requirements, and potential impact on explorer queries that access historical data.
