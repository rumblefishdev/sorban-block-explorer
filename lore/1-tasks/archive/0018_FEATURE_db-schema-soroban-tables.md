---
id: '0018'
title: 'DB schema: Soroban tables (contracts, invocations, events)'
type: FEATURE
status: done
related_adr: ['0005']
related_tasks: ['0016', '0010', '0092']
tags: [priority-high, effort-medium, layer-database]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005 + research 0092: plain SQL migrations instead of Drizzle ORM'
  - date: 2026-04-01
    status: active
    who: stkrolikiewicz
    note: 'Activated — ready to implement Soroban tables'
  - date: 2026-04-01
    status: done
    who: stkrolikiewicz
    note: >
      Implemented 3 Soroban tables in 2 migration files.
      event_interpretations removed — Event Interpreter Lambda dropped from architecture.
      All acceptance criteria verified against fresh PG 16.
      3 emerged design decisions (composite PKs, UTC partition bounds, extra FK indexes).
---

# DB schema: Soroban tables (contracts, invocations, events)

## Summary

Implement the SQL DDL for three Soroban-specific tables: `soroban_contracts`, `soroban_invocations`, and `soroban_events`. These tables model Soroban contract activity as first-class explorer entities with decoded, queryable data.

## Context

The Soroban tables form the contract-centric activity model of the explorer. Contracts are top-level entities; invocations and events are transaction children that also reference contracts.

`event_interpretations` was originally in scope but removed after the Event Interpreter Lambda was dropped from the architecture (commit 7c961e6, task 0033). Architecture docs and backlog specs that still reference `event_interpretations` or single-column PKs on Soroban activity tables are obsolete — cleanup tracked in task 0098.

## Implementation

Two plain SQL migration files in `crates/db/migrations/`:

- `0003_create_soroban_contracts.sql` — soroban_contracts table with GENERATED TSVECTOR column and GIN index
- `0004_create_soroban_activity_tables.sql` — soroban_invocations (partitioned), soroban_events (partitioned), plus initial monthly partitions (Apr-Jun 2026) with default partitions

### Cascade Chain

```
DELETE transaction
  -> CASCADE to soroban_invocations (via transaction_id FK)
  -> CASCADE to soroban_events (via transaction_id FK)
```

Deleting a contract does NOT cascade (contracts are long-lived entities).

## Acceptance Criteria

- [x] SQL DDL for soroban_contracts matches DDL with generated TSVECTOR column
- [x] SQL DDL for soroban_invocations matches DDL with monthly partitioning
- [x] SQL DDL for soroban_events matches DDL with monthly partitioning
- [x] ~~SQL DDL for event_interpretations~~ — removed (Event Interpreter dropped from architecture)
- [x] All indexes are created correctly (including GIN indexes)
- [x] Cascade chain works: delete transaction removes invocations and events
- [x] search_vector is automatically populated from metadata->>'name'
- [x] Full-text search queries work against the GIN-indexed search_vector
- [x] Initial monthly partitions are created for invocations and events
- [x] Migration files apply cleanly to a fresh PostgreSQL instance

## Implementation Notes

**Files created:**

- `crates/db/migrations/0003_create_soroban_contracts.sql`
- `crates/db/migrations/0004_create_soroban_activity_tables.sql`

**Verification:** All AC tested against fresh PostgreSQL 16 via docker-compose. Cascade chain, search_vector auto-population, full-text search, and partition existence all verified with test SQL.

## Issues Encountered

- **Test data VARCHAR(56) overflow**: Stellar contract IDs are exactly 56 characters. Test data strings initially exceeded this limit, causing insert failures during verification. Fixed by using correctly-sized test IDs.

## Design Decisions

### From Plan

1. **Two migration files, not four**: soroban_contracts in its own migration (0003) because other tables FK to it. The activity tables bundled in 0004 since they form a dependency group.

2. **Unquoted identifier style**: Follows 0002_create_operations.sql convention (plain SQL), not 0001's Drizzle-generated quoted style.

3. **Default partitions**: Added `_default` partitions for both invocations and events, matching the operations_default pattern from 0002.

### Emerged

4. **Composite PKs for partitioned tables**: PostgreSQL requires the partition key in any unique/PK constraint. Changed `soroban_invocations` and `soroban_events` PK from `(id)` to `(id, created_at)`. This is a PostgreSQL constraint, not a design preference.

5. **Explicit UTC partition bounds**: Codex review flagged that bare date literals in TIMESTAMPTZ partition bounds are session-timezone-dependent. Changed all bounds to explicit UTC: `'2026-04-01 00:00:00+00'` instead of `'2026-04-01'`.

6. **Extra indexes for FK cascade performance**: Added `idx_invocations_tx(transaction_id)` and `idx_events_tx(transaction_id)` — not in the task DDL but necessary for efficient CASCADE deletes on the FK columns.

7. **Removed event_interpretations table**: Event Interpreter Lambda was dropped from the architecture (commit 7c961e6, task 0033). Table and its indexes removed from migration 0004.

## Future Work

- Task 0022 handles automated partition creation beyond the initial Apr-Jun 2026 set

## Notes

- The GENERATED ALWAYS AS column for search_vector is written directly in SQL DDL.
- Monthly partition creation for invocations and events is covered more comprehensively in task 0022 (partition management automation). This task creates the initial set.
