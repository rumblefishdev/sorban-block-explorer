---
id: '0043'
title: 'Backend: cursor-based pagination, query parsing, and base CRUD service'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0015']
tags: [layer-backend, pagination, query-parsing, crud]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-30
    status: backlog
    who: stkrolikiewicz
    note: 'Expanded scope: added BaseCrudService and BaseCrudController to reduce boilerplate across backend modules 0045-0053'
---

# Backend: cursor-based pagination, query parsing, and base CRUD service

## Summary

Implement reusable cursor-based pagination helpers, query/filter parsing utilities, and a generic `BaseCrudService<T>` / `BaseCrudController<T>` used by all collection endpoints across the API. This includes opaque cursor encode/decode, deterministic ordering, standard response envelope, filter parsing for typed query parameters, and a base class that provides standard CRUD operations (getOne, getList, create, update, delete) for any Drizzle table. Backend entity modules (0045-0053) extend these base classes instead of reimplementing from scratch.

## Status: Backlog

**Current state:** Not started.
**Depends on:** task 0023 (NestJS API bootstrap), task 0015 (Drizzle ORM connection factory).

## Context

All collection endpoints in the explorer API use cursor-based pagination with a consistent response envelope. Cursors are opaque to clients. No total-count queries are performed. Filters are applied at the database query level before pagination, never post-query.

### API Specification

**Location:** `apps/api/src/common/pagination/`

**Standard query parameters:**

| Parameter | Type   | Default | Max | Description                  |
| --------- | ------ | ------- | --- | ---------------------------- |
| `limit`   | number | 20      | 100 | Number of items per page     |
| `cursor`  | string | null    | --  | Opaque base64-encoded cursor |

**Standard response envelope:**

```json
{
  "data": [],
  "pagination": {
    "next_cursor": "string | null",
    "has_more": true
  }
}
```

**Filter query parameters (examples used across modules):**

| Parameter                | Used By              | Description                            |
| ------------------------ | -------------------- | -------------------------------------- |
| `filter[source_account]` | Transactions         | Filter by source account ID            |
| `filter[contract_id]`    | Transactions, NFTs   | Filter by contract ID                  |
| `filter[type]`           | Transactions, Tokens | Filter by operation type or token type |
| `filter[operation_type]` | Transactions         | Filter by specific operation type      |
| `filter[code]`           | Tokens               | Filter by asset code                   |
| `filter[collection]`     | NFTs                 | Filter by NFT collection               |
| `filter[assets]`         | Liquidity Pools      | Filter by asset pair                   |
| `filter[min_tvl]`        | Liquidity Pools      | Filter by minimum TVL                  |

### Cursor Encoding

- Cursors are opaque base64-encoded strings
- Clients must never parse, construct, or assume internal cursor structure
- Cursor encodes enough state for deterministic ordering (e.g., `created_at` + `id` tie-breaking)

### Ordering

- Deterministic ordering using a primary sort key (e.g., `created_at DESC`) with `id` tie-breaking
- Stable browsing across pages without missed or duplicated items

### Behavioral Requirements

- No total-count queries -- no "Page X of Y" semantics
- All filters applied at DB query level before pagination, never post-query
- `has_more` is determined by fetching `limit + 1` rows
- `next_cursor` is null when there are no more results
- Invalid cursor values return 400 with error envelope
- Invalid limit values (negative, zero, > 100) return 400 with error envelope

### Caching

- Pagination helpers themselves are stateless; caching is handled per-endpoint at API Gateway level.

### Error Handling

```json
{
  "error": {
    "code": "INVALID_CURSOR",
    "message": "The provided cursor is invalid or expired."
  }
}
```

```json
{
  "error": {
    "code": "INVALID_LIMIT",
    "message": "Limit must be between 1 and 100."
  }
}
```

## Implementation Plan

### Step 1: Cursor encode/decode utilities

**Location:** `apps/api/src/common/pagination/cursor.ts`

Implement base64 cursor encode/decode functions. Internal cursor structure includes sort key values and tie-breaking ID. Decode validates structure and returns clear errors for malformed cursors.

### Step 2: Pagination query builder

**Location:** `apps/api/src/common/pagination/paginate.ts`

Create a reusable pagination function that accepts a Drizzle query, applies cursor-based WHERE conditions, adds ORDER BY with tie-breaking, and fetches `limit + 1` to determine `has_more`. Returns standard response envelope.

### Step 3: Filter parser

**Location:** `apps/api/src/common/filters/`

Implement a filter parsing utility that extracts `filter[key]` query parameters, validates them against allowed filter keys per endpoint, and returns typed filter objects for use in query construction.

### Step 4: NestJS validation pipes

**Location:** `apps/api/src/common/pipes/`

Create NestJS validation pipes for `limit` and `cursor` parameters with proper error mapping to 400 responses.

### Step 5: BaseCrudService

**Location:** `apps/api/src/common/base-crud.service.ts`

Generic abstract class that composes cursor pagination + Drizzle query building:

- `getOne(id)` — single record by primary key
- `getList(cursor, limit, filters?)` — cursor-paginated list using Step 2
- `create(data)` — insert with `InferInsertModel<T>`
- `update(id, data)` — partial update with `Partial<InferInsertModel<T>>`
- `delete(id)` — delete by primary key

Type-safe via Drizzle schema generics. Per-entity services extend and add custom methods.

### Step 6: BaseCrudController

**Location:** `apps/api/src/common/base-crud.controller.ts`

Generic abstract NestJS controller with full CRUD endpoints (all enabled by default, opt-out via `exclude` config):

- `GET /` — list with cursor pagination (delegates to `service.getList`)
- `GET /:id` — detail (delegates to `service.getOne`)
- `POST /` — create (delegates to `service.create`)
- `PATCH /:id` — update (delegates to `service.update`)
- `DELETE /:id` — delete (delegates to `service.delete`)

Per-entity controllers extend and configure via `exclude` to disable unneeded endpoints (e.g., most explorer modules exclude write operations). Uses validation pipes from Step 4.

### Step 7: Tests

- Unit tests for cursor encode/decode
- Unit tests for filter parser
- Integration test for BaseCrudService against local PostgreSQL (docker-compose from task 0015)

## Acceptance Criteria

- [ ] Opaque cursor encode/decode with base64 encoding
- [ ] Deterministic ordering with tie-breaking on all paginated queries
- [ ] Standard response envelope `{ data, pagination: { next_cursor, has_more } }`
- [ ] No total-count queries anywhere in pagination logic
- [ ] Filters applied at DB query level, not post-query
- [ ] `limit` validated: default 20, max 100, rejects invalid values with 400
- [ ] Invalid cursors return 400 with descriptive error
- [ ] `has_more` correctly determined by fetching limit+1
- [ ] Filter parser handles all documented filter[key] patterns
- [ ] `BaseCrudService<T>` provides getOne, getList, create, update, delete
- [ ] `BaseCrudController<T>` provides full CRUD endpoints with opt-out `exclude` config
- [ ] Type safety via Drizzle `InferSelectModel<T>` / `InferInsertModel<T>`
- [ ] Reusable across collection endpoints (BaseCrudService for 0046-0052, pagination utilities for 0045-0053)
- [ ] Unit tests for cursor and filter utilities
- [ ] Integration test for BaseCrudService against local PostgreSQL

## Notes

- Pagination utilities consumed by tasks 0045-0053 (all collection endpoints).
- `BaseCrudService` consumed by tasks 0046-0052; task 0045 (network stats) has no pagination/CRUD needs.
- The cursor structure is an internal implementation detail and must never be documented as a public contract.
- Filter keys vary per endpoint; the parser must be configurable per module.
- Search module (0053) uses cursor pagination but not BaseCrudService — it has cross-entity query patterns.
- All CRUD endpoints enabled by default; entity modules use `exclude` to disable write operations (block explorer is read-heavy).
