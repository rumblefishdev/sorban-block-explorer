---
id: '0043'
title: 'Backend: cursor-based pagination, query parsing, and base CRUD service'
type: FEATURE
status: active
related_adr: ['0005', '0008']
related_tasks: ['0023', '0094', '0042', '0046', '0050']
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
  - date: '2026-04-24'
    status: active
    who: karolkow
    note: >
      Promoted per team sync with stkrolikiewicz — 0043 must land before
      0046/0050 code is refactored to use shared pagination + CrudResource
      abstractions. 0046 shipped with inline cursor/pagination; 0050 is
      active (FilipDz) and will otherwise repeat the pattern. Scope extends
      to retroactive refactor of 0046 and sync with 0050 to adopt the shared
      abstractions.
  - date: '2026-04-24'
    status: active
    who: karolkow
    note: >
      Doc refresh: realigned envelope + error shapes to ADR 0008
      (Paginated { data, page: { cursor, limit, has_more } } and flat
      ErrorEnvelope { code, message, details }) — original 0043 draft
      predates ADR 0008. Renamed BaseCrudRepository/BaseRouter → CrudResource
      trait + crud_routes! macro throughout. Dropped create/update/delete
      language from Summary (API is read-only). Added explicit Step 8 for
      retro-refactor of 0046 onto shared helpers. Annotated filter keys with
      live/planned consumers. Corrected stale prerequisite: replaced
      references to task 0015 (pre-Rust Drizzle connection layer, superseded
      by ADR 0005) with task 0094 (Cargo workspace scaffold, which shipped
      the sqlx PgPool factory at crates/db/src/pool.rs and root
      docker-compose.yml). Net effect: no open dependencies block the start
      of implementation. No scope change beyond what was agreed on the
      2026-04-24 sync; this is a doc-only update to remove contradictions
      with ADR 0008 and the shipped 0046 implementation.
---

# Backend: cursor-based pagination, query parsing, and base CRUD service

## Summary

Implement reusable cursor-based pagination helpers, query/filter parsing utilities, and a generic `CrudResource` trait + `crud_routes!` macro used by all collection endpoints across the API. This includes opaque cursor encode/decode, deterministic ordering, the standard response envelope defined in ADR 0008, filter parsing for typed query parameters, and read-only CRUD primitives (`get_one`, `get_list`) for any sqlx table. The explorer API is read-only (data written by Ledger Processor), so no create/update/delete primitives are in scope. Backend entity modules (0045-0053) implement the trait and invoke the macro instead of reimplementing from scratch.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Envelope shapes per ADR 0008. Code in crates/api/common/.

## Status: Active

**Current state:** Not started — promoted 2026-04-24.

**Prerequisites (all satisfied):**

- **API bootstrap** (originally task 0023): delivered incidentally by task 0046 (`crates/api/src/main.rs`, `state.rs`) and task 0042 (OpenAPI infrastructure).
- **OpenAPI envelope shapes** (task 0042 + ADR 0008): `ErrorEnvelope`, `PageInfo`, `Paginated<T>` live in `crates/api/src/openapi/schemas.rs`.
- **sqlx `PgPool` factory + migrations + local PG**: delivered by task 0094 (Cargo workspace scaffold) — `crates/db/src/pool.rs`, `crates/db/src/migrate.rs`, and root `docker-compose.yml`. The original task 0015 referenced in this file's older drafts was the pre-Rust Drizzle/TypeScript connection layer and is superseded by 0094 per ADR 0005.

No open dependencies block start of implementation. The Step 7 integration test uses `crates/db::pool::create_pool` against the local PostgreSQL from `docker-compose.yml`.

## Context

All collection endpoints in the explorer API use cursor-based pagination with a consistent response envelope. Cursors are opaque to clients. No total-count queries are performed. Filters are applied at the database query level before pagination, never post-query.

The error envelope and pagination envelope shapes are fixed by [ADR 0008](../../2-adrs/0008_error-envelope-and-pagination-shape.md) and implemented in `crates/api/src/openapi/schemas.rs` as `ErrorEnvelope`, `PageInfo`, and `Paginated<T>`. This task consumes those shapes; it does not redefine them. Task 0046 (Transactions module) shipped after ADR 0008 and already uses these canonical shapes inline — its cursor/pagination/filter-parsing code will be retro-refactored onto the shared helpers delivered here.

### API Specification

**Location:** `crates/api/src/common/pagination/`

**Standard query parameters:**

| Parameter | Type   | Default | Max | Description                  |
| --------- | ------ | ------- | --- | ---------------------------- |
| `limit`   | number | 20      | 100 | Number of items per page     |
| `cursor`  | string | null    | --  | Opaque base64-encoded cursor |

**Standard response envelope** (per ADR 0008, `Paginated<T>` + `PageInfo`):

```json
{
  "data": [],
  "page": {
    "cursor": "string | null",
    "limit": 20,
    "has_more": true
  }
}
```

**Filter query parameters (examples used across modules):**

| Parameter                | Used By                         | Description                       |
| ------------------------ | ------------------------------- | --------------------------------- |
| `filter[source_account]` | Transactions (0046, live)       | Filter by source account ID       |
| `filter[contract_id]`    | Transactions (0046, live)       | Filter by contract ID             |
| `filter[operation_type]` | Transactions (0046, live)       | Filter by specific operation type |
| `filter[type]`           | Tokens (0048, planned)          | Filter by token type              |
| `filter[code]`           | Tokens (0048, planned)          | Filter by asset code              |
| `filter[collection]`     | NFTs (0049, planned)            | Filter by NFT collection          |
| `filter[assets]`         | Liquidity Pools (0051, planned) | Filter by asset pair              |
| `filter[min_tvl]`        | Liquidity Pools (0051, planned) | Filter by minimum TVL             |

> The filter parser itself must be generic — each endpoint registers the subset of keys it accepts. Only the keys marked "live" have concrete consumers today; the rest are forward-looking and will be wired in by their owning tasks.

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
- `page.has_more` is determined by fetching `limit + 1` rows
- `page.cursor` is null when there are no more results
- `page.limit` echoes the effective page size (post-validation) back to the client
- Invalid cursor values return 400 with error envelope
- Invalid limit values (negative, zero, > 100) return 400 with error envelope

### Caching

- Pagination helpers themselves are stateless; caching is handled per-endpoint at API Gateway level.

### Error Handling

All error responses use the flat `ErrorEnvelope` shape from ADR 0008 (`{ code, message, details }`). No outer `error` wrapper.

```json
{
  "code": "INVALID_CURSOR",
  "message": "The provided cursor is invalid or expired.",
  "details": null
}
```

```json
{
  "code": "INVALID_LIMIT",
  "message": "Limit must be between 1 and 100.",
  "details": { "min": 1, "max": 100, "received": 0 }
}
```

## Implementation Plan

### Step 1: Cursor encode/decode utilities

**Location:** `crates/api/src/common/cursor.rs`

Implement base64 cursor encode/decode functions. Internal cursor structure includes sort key values and tie-breaking ID. Decode validates structure and returns clear errors for malformed cursors.

### Step 2: Pagination query builder

**Location:** `crates/api/src/common/pagination.rs`

Create a reusable pagination function that accepts a sqlx query, applies cursor-based WHERE conditions, adds ORDER BY with tie-breaking, and fetches `limit + 1` to determine `has_more`. Returns `Paginated<T>` (ADR 0008) with `PageInfo { cursor, limit, has_more }` populated.

### Step 3: Filter parser

**Location:** `crates/api/src/common/filters/`

Implement a filter parsing utility that extracts `filter[key]` query parameters, validates them against allowed filter keys per endpoint, and returns typed filter objects for use in query construction.

### Step 4: axum extractors with validation

**Location:** `crates/api/src/common/extractors.rs`

Create axum extractors with validation for `limit` and `cursor` parameters. Validation failures map to 400 responses using the `ErrorEnvelope` shape from ADR 0008 (codes `INVALID_CURSOR`, `INVALID_LIMIT`).

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
- Unit tests for `limit` / `cursor` extractors (happy path + `INVALID_LIMIT` / `INVALID_CURSOR` branches; assert `ErrorEnvelope` shape)
- Integration test for `CrudResource` + `crud_routes!` against local PostgreSQL from root `docker-compose.yml`, using `crates/db::pool::create_pool` (both delivered by task 0094)

### Step 8: Retro-refactor task 0046 onto shared helpers

Task 0046 (Transactions module) shipped its own inline cursor/pagination/filter parsing before these helpers existed. As part of this task:

- Replace `crates/api/src/transactions/cursor.rs` with `common::cursor`
- Replace the cursor-predicate section of `crates/api/src/transactions/queries.rs` with `common::pagination`
- Replace `#[serde(rename = "filter[...]")]` field shims in `crates/api/src/transactions/dto.rs` with the generic filter parser from Step 3
- Implement `CrudResource for Transaction` and collapse the list/detail handlers onto `crud_routes!`, keeping any transaction-specific custom routes alongside
- Verify no change to the wire contract of `/transactions` endpoints — existing integration tests must stay green without edits to expected response bodies

## Acceptance Criteria

- [ ] Opaque cursor encode/decode with base64 encoding
- [ ] Deterministic ordering with tie-breaking on all paginated queries
- [ ] Response envelope matches ADR 0008: `Paginated<T> { data, page: PageInfo { cursor, limit, has_more } }`
- [ ] Error responses match ADR 0008: flat `ErrorEnvelope { code, message, details }` (no outer `error` wrapper)
- [ ] No total-count queries anywhere in pagination logic
- [ ] Filters applied at DB query level, not post-query
- [ ] `limit` validated: default 20, max 100, rejects invalid values with 400 + `INVALID_LIMIT`
- [ ] Invalid cursors return 400 + `INVALID_CURSOR`
- [ ] `page.has_more` correctly determined by fetching limit+1
- [ ] `page.cursor` is `None` on the last page
- [ ] Filter parser handles all documented `filter[key]` patterns, configurable per endpoint
- [ ] `CrudResource` trait provides `get_one`, `get_list` with compile-time checked sqlx queries
- [ ] `crud_routes!` macro generates axum Router with `#[utoipa::path]` annotations
- [ ] Type safety via `sqlx::FromRow` + `utoipa::ToSchema` derives
- [ ] Reusable across collection endpoints (CrudResource trait for 0046-0052, pagination utilities for 0045-0053)
- [ ] Task 0046 (Transactions) refactored onto shared helpers without wire-contract change
- [ ] Unit tests for cursor, filter parser, and extractors (including ADR 0008 error shape assertions)
- [ ] Integration test for `CrudResource` + `crud_routes!` against local PostgreSQL

## Notes

- Pagination utilities consumed by tasks 0045-0053 (all collection endpoints).
- `CrudResource` trait consumed by tasks 0046-0052; task 0045 (network stats) has no pagination/CRUD needs.
- Task 0046 (Transactions) is already shipped with inline cursor/pagination/filter parsing; Step 8 retro-refactors it onto the shared helpers. Task 0050 (Contracts, in-flight under FilipDz) will adopt the shared helpers from the start — coordination with FilipDz/stkrolikiewicz required before 0050 merges to avoid a second inline implementation.
- All wire shapes (`Paginated<T>`, `PageInfo`, `ErrorEnvelope`) are fixed by ADR 0008. Any deviation requires a superseding ADR, not an inline override in this task.
- The cursor payload structure is an internal implementation detail and must never be documented as a public contract.
- Filter keys vary per endpoint; the parser must be configurable per module.
- Search module (0053) uses cursor pagination but not `CrudResource` — it has cross-entity query patterns.
- Explorer API is read-only (data written by Ledger Processor) — no create/update/delete endpoints needed.
