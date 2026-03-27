---
id: '0007'
title: 'Research: Drizzle ORM with PostgreSQL partitioning and advanced features'
type: RESEARCH
status: completed
assignee: 'stkrolikiewicz'
related_adr: []
related_tasks: ['0008', '0009', '0010', '0011', '0012']
tags: [priority-high, effort-small, layer-research]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
  - date: '2026-03-26'
    status: active
    who: stkrolikiewicz
    note: 'Activated for research.'
  - date: '2026-03-26'
    status: completed
    who: stkrolikiewicz
    note: >
      Completed research. 14-feature compatibility matrix, version matrix
      verified against drizzle-orm 0.45.1 / drizzle-kit 0.31.10.
      4 research notes, 1 synthesis, 14 archived sources.
      All 9 acceptance criteria met. PR #17.
---

# Research: Drizzle ORM with PostgreSQL partitioning and advanced features

## Summary

Investigate Drizzle ORM compatibility with PostgreSQL features required by the block explorer schema: range partitioning, foreign keys with ON DELETE CASCADE on partitioned tables, tsvector generated columns, GIN indexes on JSONB and tsvector, CHECK constraints, and Drizzle Kit migration generation for these features. This research must produce a compatibility matrix and documented workarounds for any gaps.

## Status: Completed

## Acceptance Criteria

- [x] Compatibility matrix: each PostgreSQL feature vs Drizzle ORM native support (yes/no/partial) — `notes/S-compatibility-matrix.md`
- [x] Documented workaround for each unsupported feature — `notes/R-partitioning-support.md`, `notes/R-gin-tsvector-check-jsonb.md`
- [x] Drizzle schema definition examples for partitioned tables (or confirmation that raw SQL is needed) — `notes/R-partitioning-support.md`
- [x] Foreign key + CASCADE on partitioned tables: working approach documented — `notes/R-partitioning-support.md` (per-table analysis)
- [x] tsvector generated column: working approach documented — `notes/R-gin-tsvector-check-jsonb.md` (with JSONB arrow operator)
- [x] GIN index definition: working approach documented — `notes/R-gin-tsvector-check-jsonb.md`
- [x] CHECK constraint: working approach documented — `notes/R-gin-tsvector-check-jsonb.md`
- [x] JSONB column TypeScript typing approach documented — `notes/R-gin-tsvector-check-jsonb.md`
- [x] Drizzle Kit migration workflow documented for mixed native + raw SQL features — `notes/R-drizzle-kit-migrations.md`

## Implementation Notes

### Research Notes (notes/)

| File                            | Type      | Content                                                                                            |
| ------------------------------- | --------- | -------------------------------------------------------------------------------------------------- |
| `R-partitioning-support.md`     | Research  | Partitioning not supported, workaround via custom migrations, FK + composite PK per-table analysis |
| `R-gin-tsvector-check-jsonb.md` | Research  | GIN (native), tsvector (customType + generatedAlwaysAs), CHECK (partial), JSONB (.$type<T>())      |
| `R-drizzle-kit-migrations.md`   | Research  | generate + migrate workflow, custom SQL injection, --custom flag                                   |
| `S-compatibility-matrix.md`     | Synthesis | 14-feature compatibility matrix, version matrix, migration strategy, impact on tasks 0015-0022     |

### Archived Sources (sources/)

14 source files from Drizzle docs (7), GitHub issues/PRs (5), PostgreSQL docs (1), Drizzle guides (1). All URLs verified HTTP 200.

### Key Findings

1. **Partitioning: NOT supported** — define tables in schema for type safety, hand-write PARTITION BY RANGE in migration SQL. Issue #2854 open, no ETA.
2. **GIN indexes: fully native** — `index().using('gin', column)`
3. **tsvector generated columns: native** — `customType` + `generatedAlwaysAs()` callback with JSONB arrow operator
4. **CHECK constraints: partial** — must use array syntax, drizzle-kit may skip in migrations
5. **JSONB typing: native** — `.$type<T>()` for compile-time safety
6. **Migration workflow: generate + edit + --custom** for mixed native/raw SQL

## Design Decisions

### From Plan

1. **Use `generate` + `migrate` workflow, not `push`** — partitioned tables require hand-written SQL in migration files, which `push` cannot support.

### Emerged

2. **Composite PKs required for all partitioned tables** — PostgreSQL requires partition key in PK. This affects PK design: `(id, transaction_id)` for operations, `(id, created_at)` for the 3 monthly-partitioned tables.

3. **Forward-compatible code examples** — Used `sql` template callbacks (not string literals) in all examples to ensure compatibility with v1.0.0-beta.12+ API change.

4. **Array syntax for CHECK is critical** — Object syntax `(t) => ({...})` silently drops CHECK from migrations. Documented as #1 pitfall.

## Context

The block explorer schema uses several advanced PostgreSQL features that are not universally supported by ORMs. Drizzle ORM is the chosen data access layer, and its compatibility with these features must be validated before implementation begins. The schema contains 4 partitioned tables, approximately 10 JSONB columns, GIN indexes, full-text search via tsvector, and CHECK constraints.

## Notes

- The schema evolution rules state: avoid replacing explicit relational structure with oversized generic JSON blobs. Understanding how Drizzle handles JSONB typing is important for maintaining this discipline.
- Partition lifecycle (create ahead of time, drop operationally) is an infrastructure concern, not an ORM concern. But the ORM must at least not break when querying partitioned tables.
- The ingestion path uses per-ledger database transactions with batch insertion of child rows. Drizzle transaction support and batch insert performance are relevant but outside the scope of this specific research task.
