---
id: '0043'
title: 'Backend: cursor-based pagination, query parsing, and base CRUD service'
type: FEATURE
status: backlog
related_adr: ['0005']
related_tasks: ['0023', '0015', '0092']
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
    note: 'Expanded scope: added BaseCrudRepository and BaseRouter to reduce boilerplate across backend modules 0045-0053'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: axum → Rust (axum + utoipa + sqlx)'
---

# Backend: cursor-based pagination, query parsing, and base CRUD service

## Summary

Implement reusable cursor-based pagination helpers, query/filter parsing utilities, and a generic `BaseCrudRepository<T>` / `BaseRouter<T>` used by all collection endpoints across the API. This includes opaque cursor encode/decode, deterministic ordering, standard response envelope, filter parsing for typed query parameters, and a base class that provides standard CRUD operations (getOne, getList, create, update, delete) for any sqlx table. Backend entity modules (0045-0053) extend these base classes instead of reimplementing from scratch.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Backlog

**Current state:** Not started.
**Depends on:** task 0023 (API bootstrap), task 0015 (sqlx connection factory).

## Context

All collection endpoints in the explorer API use cursor-based pagination with a consistent response envelope. Cursors are opaque to clients. No total-count queries are performed. Filters are applied at the database query level before pagination, never post-query.

### API Specification

**Location:** `crates/api/src/common/pagination/`

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

**Location:** `crates/api/src/common/cursor.rs`

Implement base64 cursor encode/decode functions. Internal cursor structure includes sort key values and tie-breaking ID. Decode validates structure and returns clear errors for malformed cursors.

### Step 2: Pagination query builder

**Location:** `crates/api/src/common/pagination.rs`

Create a reusable pagination function that accepts a sqlx query, applies cursor-based WHERE conditions, adds ORDER BY with tie-breaking, and fetches `limit + 1` to determine `has_more`. Returns standard response envelope.

### Step 3: Filter parser

**Location:** `crates/api/src/common/filters/`

Implement a filter parsing utility that extracts `filter[key]` query parameters, validates them against allowed filter keys per endpoint, and returns typed filter objects for use in query construction.

### Step 4: axum extractors with validation

**Location:** `crates/api/src/common/extractors.rs`

Create axum extractors with validation for `limit` and `cursor` parameters with proper error mapping to 400 responses.

### Step 5: CRUD trait + macro

**Location:** `crates/api/src/common/crud.rs`

Rust trait `CrudResource` + `macro_rules! crud_routes` that composes cursor pagination + sqlx queries:

- `get_one(id)` — single record by primary key via `sqlx::query_as`
- `get_list(cursor, limit, filters?)` — cursor-paginated list using Step 2
- Type-safe via `sqlx::FromRow` + `utoipa::ToSchema` derives
- Per-resource modules implement the trait and add custom query methods
- Note: API is read-only (data written by Ledger Processor), so create/update/delete not needed for explorer endpoints

### Step 6: Route builder macro

**Location:** `crates/api/src/common/crud.rs` (same file as Step 5)

`crud_routes!` macro generates axum `Router` with read endpoints per resource:

- `GET /` — list with cursor pagination (uses `CrudResource::get_list`)
- `GET /{id}` — detail (uses `CrudResource::get_one`)
- `#[utoipa::path]` annotations auto-generated per endpoint
- Concrete paginated response type generated via Rust type alias (utoipa 5.x pattern)

Per-resource modules invoke the macro and can add custom routes alongside generated ones.

### Step 7: Tests

- Unit tests for cursor encode/decode
- Unit tests for filter parser
- Integration test for BaseCrudRepository against local PostgreSQL (docker-compose from task 0015)

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
- [ ] `CrudResource` trait provides `get_one`, `get_list` with compile-time checked sqlx queries
- [ ] `crud_routes!` macro generates axum Router with `#[utoipa::path]` annotations
- [ ] Type safety via `sqlx::FromRow` + `utoipa::ToSchema` derives
- [ ] Reusable across collection endpoints (CrudResource trait for 0046-0052, pagination utilities for 0045-0053)
- [ ] Unit tests for cursor and filter utilities
- [ ] Integration test for BaseCrudRepository against local PostgreSQL

## Notes

- Pagination utilities consumed by tasks 0045-0053 (all collection endpoints).
- `CrudResource` trait consumed by tasks 0046-0052; task 0045 (network stats) has no pagination/CRUD needs.
- The cursor structure is an internal implementation detail and must never be documented as a public contract.
- Filter keys vary per endpoint; the parser must be configurable per module.
- Search module (0053) uses cursor pagination but not CrudResource — it has cross-entity query patterns.
- Explorer API is read-only (data written by Ledger Processor) — no create/update/delete endpoints needed.
