---
id: '0043'
title: 'Backend: cursor-based pagination helpers and query parsing'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023']
tags: [layer-backend, pagination, query-parsing]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: cursor-based pagination helpers and query parsing

## Summary

Implement reusable cursor-based pagination helpers and query/filter parsing utilities used by all collection endpoints across the API. This includes opaque cursor encode/decode, deterministic ordering, standard response envelope, and filter parsing for typed query parameters.

## Status: Backlog

**Current state:** Not started. Depends on task 0023 (NestJS API bootstrap).

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

### Step 1: Cursor Encode/Decode Utilities

Implement base64 cursor encode/decode functions. Internal cursor structure should include sort key values and tie-breaking ID. Decode must validate structure and return clear errors for malformed cursors.

### Step 2: Pagination Query Builder

Create a reusable pagination query builder that accepts a Drizzle query, applies cursor-based WHERE conditions, adds ORDER BY with tie-breaking, and fetches `limit + 1` to determine `has_more`.

### Step 3: Response Envelope Builder

Build a helper that takes raw query results and produces the standard `{ data, pagination: { next_cursor, has_more } }` response envelope.

### Step 4: Filter Parser

Implement a filter parsing utility that extracts `filter[key]` query parameters, validates them against allowed filter keys per endpoint, and returns typed filter objects for use in query construction.

### Step 5: Validation Pipes

Create NestJS validation pipes for `limit` and `cursor` parameters with proper error mapping to 400 responses.

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
- [ ] Reusable across all collection endpoints

## Notes

- This module is consumed by tasks 0046-0053 (all collection endpoints).
- The cursor structure is an internal implementation detail and must never be documented as a public contract.
- Filter keys vary per endpoint; the parser must be configurable per module.
