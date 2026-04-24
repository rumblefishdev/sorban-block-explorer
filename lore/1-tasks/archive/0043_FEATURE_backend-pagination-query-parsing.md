---
id: '0043'
title: 'Backend: cursor-based pagination, query parsing, and base CRUD service'
type: FEATURE
status: completed
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
  - date: '2026-04-24'
    status: completed
    who: karolkow
    note: >
      Implemented all 8 steps. 6 new files under crates/api/src/common/
      (cursor, errors, extractors, filters, pagination, mod). 0046
      retro-refactored onto shared helpers: transactions/cursor.rs deleted,
      queries.rs cursor predicate replaced by push_ts_id_cursor_predicate,
      handlers.rs switched to Pagination extractor + filters::strkey +
      filters::parse_enum + errors::* + finalize_ts_id_page + into_envelope.
      Wire contract of /v1/transactions unchanged. Post-implementation
      audit against 0045-0053 roadmap showed 0/13 planned list endpoints
      fit the CrudResource trait shape (hardcoded TsIdCursor, no filter
      slot, no enrichment hook). stkrolikiewicz agreed to remove the trait
      + macro before merge. common/crud.rs relocated to
      .trash/api_common_crud.rs; re-extraction tracked under backlog
      task 0166. AC 11/12/14/17 deferred. Retained common/* helpers
      (cursor, errors, extractors, filters, pagination) adopt cleanly
      across all planned endpoints. Final test tally: 48 api-crate tests
      passing (+14 new helper coverage + 5 integration). No
      docs/architecture changes needed (internal-helpers refactor).
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

- [x] Opaque cursor encode/decode with base64 encoding — `common::cursor::{encode, decode, TsIdCursor}`
- [x] Deterministic ordering with tie-breaking on all paginated queries — `push_ts_id_cursor_predicate` enforces `(ts_col, id_col) < ($ts, $id)` against the existing `ORDER BY t.created_at DESC, t.id DESC`
- [x] Response envelope matches ADR 0008: `Paginated<T> { data, page: PageInfo { cursor, limit, has_more } }` — consumed from `openapi::schemas`, assembled by `into_envelope`
- [x] Error responses match ADR 0008: flat `ErrorEnvelope { code, message, details }` (no outer `error` wrapper) — `common::errors` helpers
- [x] No total-count queries anywhere in pagination logic — `finalize_page` uses the limit+1 peek
- [x] Filters applied at DB query level, not post-query — `queries::fetch_list` JOIN/WHERE construction unchanged
- [x] `limit` validated: default 20, max 100, rejects invalid values with 400 + `INVALID_LIMIT` — `LimitConfig::DEFAULT` + `validate_limit`
- [x] Invalid cursors return 400 + `INVALID_CURSOR` — `decode_cursor`
- [x] `page.has_more` correctly determined by fetching limit+1 — `finalize_page`
- [x] `page.cursor` is `None` on the last page — `finalize_page` branch
- [x] Filter parser handles all documented `filter[key]` patterns, configurable per endpoint — `filters::strkey`, `filters::parse_enum` (live consumers: `filter[source_account]`, `filter[contract_id]`, `filter[operation_type]`; remaining keys are forward-looking for 0048/0049/0051)
- [ ] `CrudResource` trait provides `get_one`, `get_list` with compile-time checked sqlx queries — **deferred to [0166](../backlog/0166_FEATURE_crud-resource-scaffolding.md)** per post-audit (2026-04-24). See Post-audit Note below.
- [ ] `crud_routes!` macro generates axum Router with `#[utoipa::path]` annotations — **deferred to [0166](../backlog/0166_FEATURE_crud-resource-scaffolding.md)** per post-audit (2026-04-24).
- [x] Type safety via `sqlx::FromRow` + `utoipa::ToSchema` derives — present on `openapi::schemas::{Paginated, ErrorEnvelope, PageInfo}` and consumed through the retained `common/*` helpers.
- [ ] Reusable across collection endpoints (CrudResource trait for 0046-0052, pagination utilities for 0045-0053) — **trait deferred to [0166](../backlog/0166_FEATURE_crud-resource-scaffolding.md)**; pagination utilities retained and in use by 0046 (retro-refactored in this task), adoption planned across 0047-0053.
- [x] Task 0046 (Transactions) refactored onto shared helpers without wire-contract change — `/v1/transactions` response body identical; existing api-crate unit tests unchanged (3 obsolete `transactions::cursor` tests dropped — covered by `common::cursor` tests)
- [x] Unit tests for cursor, filter parser, and extractors (including ADR 0008 error shape assertions) — see `common::{cursor,filters,extractors,pagination}::tests`
- [ ] Integration test for `CrudResource` + `crud_routes!` against local PostgreSQL — **deferred to [0166](../backlog/0166_FEATURE_crud-resource-scaffolding.md)** (trait itself deferred). End-to-end coverage against local PostgreSQL is retained via `tests_integration.rs`: 4 validation tests (no DB) + 1 DB-gated end-to-end envelope test through `/v1/transactions` exercise every retained helper (`Pagination<TsIdCursor>`, `filters::strkey`, `filters::parse_enum`, `errors::*`, `finalize_ts_id_page`, `into_envelope`).

## Docs updated (per ADR 0032)

- `docs/architecture/**` — **N/A**: internal-helpers refactor with no wire-contract, schema, endpoint, or infrastructure change. `/v1/transactions` request + response unchanged; no new endpoints added. The only docs-visible surface is the OpenAPI spec at `/api-docs-json`, which is generated from utoipa annotations — spec shapes for `Paginated`/`ErrorEnvelope` are unchanged.

## Implementation Notes

**New modules under `crates/api/src/common/`** (all with in-module `#[cfg(test)] mod tests`):

- `cursor.rs` — generic `encode<P: Serialize>` / `decode<P: DeserializeOwned>` over base64url-wrapped JSON. `TsIdCursor { ts, id }` is the default payload; consumers with bespoke ordering (sequence number, hash prefix) define their own.
- `errors.rs` — canonical code constants (`INVALID_CURSOR`, `INVALID_LIMIT`, `INVALID_FILTER`, `NOT_FOUND`, `DB_ERROR`) + `bad_request`, `bad_request_with_details`, `not_found`, `internal_error`, `envelope` builders returning `Response`.
- `extractors.rs` — `Pagination<P>` axum extractor (native `FromRequestParts`) plus `resolve`/`resolve_with` for handlers carrying their own `Query<ListParams>` DTO. `LimitConfig { default, max }` with a const `DEFAULT` (20/100) per ADR 0008.
- `filters.rs` — `strkey(value, prefix, filter_key)` (shape-only RFC 4648 base32 check) and `parse_enum::<T: FromStr>` both returning `Result<T, Response>` so handlers can `?`-propagate into 400 envelopes.
- `pagination.rs` — `finalize_page<Row>`, `finalize_ts_id_page<Row>`, `into_envelope<T>`, `push_ts_id_cursor_predicate`. The `push_ts_id_cursor_predicate` helper is consumed by `transactions/queries.rs` for the `(created_at, id)` ordering used across the DB schema.
- ~~`crud.rs` — `CrudResource` trait + `crud_routes!` macro.~~ **Removed before merge** per post-audit (see Design Decisions → Emerged #6 and the Post-audit Note). Moved to `.trash/api_common_crud.rs`. Re-extraction tracked under backlog [0166](../backlog/0166_FEATURE_crud-resource-scaffolding.md).

**Retro-refactor of 0046 (`crates/api/src/transactions/`):**

- `transactions/cursor.rs` — deleted (moved to `.trash/`). Module declaration removed from `mod.rs`.
- `transactions/dto.rs` — `ListParams` no longer carries `limit`/`cursor` fields; those are documented via `#[utoipa::path(params(...))]` on the list handler and read by the sibling `Pagination<TsIdCursor>` extractor.
- `transactions/queries.rs` — `ResolvedListParams.cursor` changed from `Option<(DateTime<Utc>, i64)>` to `Option<TsIdCursor>`; cursor predicate emitted via `push_ts_id_cursor_predicate`; `parse_op_type` removed (inlined to `filters::parse_enum::<OperationType>` at the call site).
- `transactions/handlers.rs` — `err()` helper and `is_valid_strkey()` removed; error returns use `errors::*`; StrKey filters use `filters::strkey`; enum filters use `filters::parse_enum`; `limit` / `cursor` parsed by `Pagination` extractor; `has_more` + cursor assembly uses `finalize_ts_id_page` + `into_envelope`.

**Integration test (`crates/api/src/tests_integration.rs`, `#[cfg(test)]`):** 4 unconditional validation tests prove the `Pagination` extractor + `filters::*` + `errors::*` wire through the real axum request stack to the canonical 400 envelope (checking the `code` and `details.filter` / `details.received` keys specifically). 1 DATABASE_URL-gated test calls `GET /v1/transactions?limit=3` against the real pool and asserts the envelope shape (`data` array, `page.limit`, `page.has_more`, `page.cursor` optional). Follows the same skip-on-unset pattern as `crates/indexer/tests/persist_integration.rs`.

**Test tally (post-audit, crud.rs removed):** 48 passing in `cargo test -p api --bin api` (43 unit + 5 integration). 5 ignored (existing AWS-live network tests). No previously-passing test was modified to match new behaviour. The 5 `common::crud::tests::*` tests that used to bring the total to 53 were removed alongside `crud.rs`.

## Design Decisions

### From Plan

1. **Opaque cursors, base64url(JSON)** — per ADR 0008. `TsIdCursor` is the default `(ts, id)` payload; the underlying `encode/decode` are generic so bespoke payloads stay idiomatic.
2. **`limit + 1` peek for `has_more`** — per ADR 0008, avoids a `COUNT(*)` per list call.
3. **Flat `ErrorEnvelope`** — per ADR 0008; no outer `error` wrapper. All error helpers return `Response` directly so handlers `?`-propagate.
4. **Shape-only StrKey validation, no CRC** — catches the common typo / wrong-prefix cases that would otherwise silently return empty pages, without pulling in a CRC dependency. The full CRC is re-checked at DB lookup time via `accounts.account_id` / `soroban_contracts.contract_id`.
5. **Filter DSL parsing stays in serde** — each endpoint's `ListParams` keeps `#[serde(rename = "filter[key]")]` on its own fields; the shared module only owns _validation_ of values (StrKey shape, enum-name recognition). This keeps each endpoint's accepted key set explicit and type-checked at the DTO without requiring a generic filter registry.

### Emerged

6. **CrudResource trait + crud_routes! macro — built, audited, removed before merge.** My pre-implementation recommendation was to skip the trait + macro (rule of three not met — only transactions would implement it at this moment, and transactions has a custom-enough post-fetch enrichment path that forcing it through the trait would lose expressivity). User chose to build per AC literal wording. Post-implementation audit against the 0045-0053 roadmap (see **Post-audit Note** below) showed **0/13 planned list endpoints** fit the trait's hardcoded shape (`TsIdCursor`-only ordering, no filter params, no enrichment hook). stkrolikiewicz confirmed on 2026-04-24 to remove ("wywal"). Trait + macro moved to `.trash/api_common_crud.rs`; AC 11/12/14/17 marked **deferred to [0166](../backlog/0166_FEATURE_crud-resource-scaffolding.md)**. 0166 carries the trigger condition (two real simple-shape consumers must land first) and the rewrite note (trait must be generic over cursor payload type, not hardcoded to `TsIdCursor`).

#### Post-audit Note (2026-04-24)

After the trait + macro landed in the first implementation round, a roadmap audit was run against the tech-design specs of every planned list endpoint (0045-0053). The mapping:

| Task | Endpoint                             | Cursor payload      | Filters                                     | Enrichment                       | Fits trait?                                         |
| ---- | ------------------------------------ | ------------------- | ------------------------------------------- | -------------------------------- | --------------------------------------------------- |
| 0045 | `/network-stats`                     | —                   | —                                           | aggregates                       | N/A — single stats endpoint, not a list             |
| 0046 | `/transactions`                      | `(ts, id)`          | source_account, contract_id, operation_type | S3 memo fetch                    | ❌ enrichment does not fit `into_item`              |
| 0047 | `/ledgers`                           | **`sequence`**      | —                                           | cache-control headers            | ❌ wrong cursor type (trait hardcodes `TsIdCursor`) |
| 0047 | `/ledgers/{seq}/transactions`        | `(ts, id)`          | —                                           | nested                           | ❌ nested route                                     |
| 0048 | `/accounts/{id}/transactions`        | `id`                | —                                           | nested                           | ❌ nested route                                     |
| 0049 | `/assets`                            | `id`                | `filter[type]`, `filter[code]`              | type unification                 | ❌ trait has no filter slot                         |
| 0049 | `/assets/{id}/transactions`          | `id`                | —                                           | join through ops/events          | ❌ custom join                                      |
| 0050 | `/contracts/*`                       | varied              | complex                                     | interfaces/invocations/events    | ❌ multi-endpoint                                   |
| 0051 | `/nfts`                              | `id`                | `filter[collection]`, `filter[contract_id]` | —                                | ❌ trait has no filter slot                         |
| 0051 | `/nfts/{id}/transfers`               | `id`                | —                                           | derived from events, not a table | ❌ custom                                           |
| 0052 | `/liquidity-pools`                   | `id`                | `filter[assets]`, `filter[min_tvl]`         | —                                | ❌ trait has no filter slot                         |
| 0052 | `/liquidity-pools/{id}/transactions` | `id`                | —                                           | nested                           | ❌ nested                                           |
| 0053 | `/search`                            | cross-entity custom | custom                                      | custom                           | ❌                                                  |

**0/13 fit.** Two structural problems:

1. **Cursor payload is not `(ts, id)` for most resources** — specs use `{"seq":N}`, `{"id":N}`. Trait hardcodes `TsIdCursor`. Any real consumer would need to bypass the trait or force trait evolution (`type Cursor` associated type + macro adaptation).
2. **Filters are the rule, not the exception** — 4 of 5 non-transactions list endpoints accept `filter[...]` params. Trait's `get_list(state, cursor, limit)` has no filter slot. Adding one requires `type Filters` + macro changes + utoipa gymnastics.

In contrast, the retained `common/*` helpers are **orthogonal** to cursor payload:

- `cursor::encode<P: Serialize> / decode<P: DeserializeOwned>` — generic over any payload.
- `finalize_page<Row>` — generic, caller supplies the cursor-mapping closure.
- `finalize_ts_id_page<Row>` — convenience for the `(ts, id)` case (used by 0046).
- `Pagination<P>` extractor — generic over payload. Ledgers compose `Pagination<SequenceCursor>`, assets/NFTs/pools compose `Pagination<IdCursor>`, transactions composes `Pagination<TsIdCursor>`.
- `filters::strkey`, `filters::parse_enum` — reusable by every filtered endpoint.
- `errors::*` — envelope builders reusable across all failure paths.
- `push_ts_id_cursor_predicate` — specific to `(ts, id)`; sibling helpers for `sequence` / `id`-only ordering will be added as those resources land, mirroring this one.

Every planned endpoint (12/13) composes the helpers directly. The trait adds complexity without catching any endpoint's shape.

stkrolikiewicz agreed to remove. Trait + macro relocated to `.trash/api_common_crud.rs`; re-extraction tracked under backlog task [0166](../backlog/0166_FEATURE_crud-resource-scaffolding.md) with a trigger condition of two real simple-shape consumers and a requirement that the rewrite generalise over cursor payload type.

7. **Transactions does not implement CrudResource.** The list handler's post-fetch memo enrichment (concurrent S3 ledger fetch → `extract_e3_memo` merge) doesn't fit `CrudResource::into_item(row) -> item` — it requires cross-row state (the ledger map). Forcing it through the trait would either bloat the trait with an "enrichment phase" hook that no other resource needs, or require the trait to expose the raw `Vec<Row>` before mapping, which defeats the point. Transactions consumes the low-level helpers (`Pagination`, `finalize_ts_id_page`, `into_envelope`, `push_ts_id_cursor_predicate`, `filters::*`, `errors::*`) directly.

8. **Dropped `limit`/`cursor` from `ListParams` rather than dual-sourcing them.** The axum `Query<ListParams>` extractor tolerates unknown fields, so having a sibling `Pagination` extractor alongside works cleanly. `limit`/`cursor` are documented via `#[utoipa::path(params(...))]` inline literals on the handler rather than on the DTO — one source of truth, no risk of the DTO and extractor disagreeing on validation semantics.

9. **`utoipa::path(body = ...)` macro quirk in `crud_routes!`** — utoipa's attribute proc macro uses the _last path segment_ as the identifier in its generated `ToSchema` wiring, so `body = $crate::openapi::schemas::Paginated<$item>` failed to resolve. Fixed by emitting a `use $crate::openapi::schemas::{ErrorEnvelope, Paginated};` at the top of the macro expansion and referring to the types unqualified in `body = ...`.

10. **Integration test placed inside `src/` as `#[cfg(test)] mod tests_integration;`**, not under `crates/api/tests/`. The api crate is binary-only (`[[bin]]` with `src/main.rs`) — a proper `tests/` directory would require adding a `lib.rs` with re-exports of `common`, `transactions`, `state`, and `stellar_archive` just to make them reachable from the integration test. Keeping the test inside `src/` avoids that crate-surface change; the DATABASE_URL gate makes it behave like a conventional integration test (runs when the DB is up, skips cleanly when it is not).

## Issues Encountered

- **StrKey test fixtures**: initial `VALID_G` / `VALID_C` constants in `filters::tests` were 52 chars, not 56 — miscounted the body padding. Caught by the very first `strkey()` call returning an error in tests; fixed by extending the constants and adding an explicit `test_constants_are_56_chars` sanity test.
- **`has_where` tracking preserved manually in `queries::fetch_list`**: the shared `push_ts_id_cursor_predicate` helper does not manage the `WHERE`/`AND` glue — the caller is responsible, consistent with how dynamic filters already build their own glue. This was an intentional scope choice (the glue logic is specific to the set of filters each query allows) and is documented in `pagination.rs` doccomment.
- **`clippy::result_large_err` on the pre-push hook**: axum's `Response` type is ~128 bytes, so `Result<T, Response>` trips the lint. Boxing would cascade through every call site and kill the ergonomics of `?`-propagation into envelope responses. Resolved with `#![allow(clippy::result_large_err)]` at the top of `common/extractors.rs` and `common/filters.rs` — scoped to the two modules that own the `Result<_, Response>` boundary. Documented in the commit message and in the module docs.
- **Historical (since removed): CrudResource trait surfaced a set of sharp edges** — `async_trait` crate misuse (resolved by native 2024 async-in-trait), `utoipa::path(body = ...)` treating the last path segment as the type identifier (resolved by injecting a `use` in the macro expansion), and `private_interfaces` warnings on the test smoke `WidgetResource`/`WidgetState`. These are no longer part of the shipped code but are relevant historical context if task 0166 re-extracts the trait — see the Post-audit Note under Design Decisions → Emerged.

## Future Work

- **Re-extract CrudResource trait + crud_routes! macro when first simple consumer lands** — tracked under backlog task [0166](../backlog/0166_FEATURE_crud-resource-scaffolding.md). Trigger: two resources with simple-shape lists (e.g. `/ledgers` with `SequenceCursor` + zero filters + zero enrichment) must exist before re-extracting. The rewrite must generalise the cursor payload (associated type, not hardcoded `TsIdCursor`) — the main reason the original drop didn't fit the roadmap.
- **Add `push_sequence_cursor_predicate` / `push_id_cursor_predicate` siblings to `common/pagination.rs`** when 0047 (ledgers, `sequence`-ordered) and 0049/0051/0052 (assets/NFTs/pools, `id`-ordered) land. These are trivial copies of `push_ts_id_cursor_predicate` tuned to their respective cursor payloads. Tracked implicitly by the owning resource tasks — no separate backlog item needed.
- **Coordinate with FilipDz on 0050 (Contracts)**: 0050 should consume `Pagination<TsIdCursor>` + `finalize_ts_id_page` + `filters::*` from the start rather than re-derive them. Call-out on 2026-04-24 sync covered this; no backlog task needed — the 0050 PR review is the enforcement point.

## Notes

- Pagination utilities consumed by tasks 0045-0053 (all collection endpoints).
- `CrudResource` trait consumed by tasks 0046-0052; task 0045 (network stats) has no pagination/CRUD needs.
- Task 0046 (Transactions) is already shipped with inline cursor/pagination/filter parsing; Step 8 retro-refactors it onto the shared helpers. Task 0050 (Contracts, in-flight under FilipDz) will adopt the shared helpers from the start — coordination with FilipDz/stkrolikiewicz required before 0050 merges to avoid a second inline implementation.
- All wire shapes (`Paginated<T>`, `PageInfo`, `ErrorEnvelope`) are fixed by ADR 0008. Any deviation requires a superseding ADR, not an inline override in this task.
- The cursor payload structure is an internal implementation detail and must never be documented as a public contract.
- Filter keys vary per endpoint; the parser must be configurable per module.
- Search module (0053) uses cursor pagination but not `CrudResource` — it has cross-entity query patterns.
- Explorer API is read-only (data written by Ledger Processor) — no create/update/delete endpoints needed.
