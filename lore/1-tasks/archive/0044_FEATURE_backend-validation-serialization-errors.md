---
id: '0044'
title: 'Backend: request validation, response serialization, error mapping'
type: FEATURE
status: completed
related_adr: ['0005', '0008', '0029']
related_tasks: ['0023', '0014', '0043', '0046', '0050', '0092']
tags: [layer-backend, validation, serialization, error-handling]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: axum → Rust (axum + utoipa + sqlx)'
  - date: '2026-04-27'
    status: active
    who: karolkow
    note: >
      Promoted. Spec realign required pre-implementation: original draft
      (March 2026) predates ADR 0008 (envelope shape) and the 0043+0046
      delivery of common/* validation/error helpers. Per audit, ~80% of
      the originally-scoped surface is already shipped via task 0043
      (extractors, filters, errors envelope builders) and task 0046
      (parse_error field, unknown operation_type fallback). Realigned
      scope reduces to two concrete deliverables — see body for details.
  - date: '2026-04-27'
    status: active
    who: karolkow
    note: >
      Doc realign (no scope change). Error envelope examples
      switched to ADR 0008 flat shape (was: outer-`error`-wrapper draft
      predating ADR 0008); error-code table switched to lowercase canonical
      codes shipped in `crates/api/src/common/errors.rs` under task 0043
      (`invalid_limit` / `invalid_cursor` / `invalid_query` /
      `invalid_filter` / `not_found` / `db_error`). Added pointer note on
      parse_error example clarifying that the live wire shape is the
      ADR 0029 / task 0150 `E3Response<T>` wrapper. Body otherwise
      unchanged.
  - date: '2026-04-28'
    status: active
    who: karolkow
    note: >
      Step 6 (graceful degradation verification) shipped on this branch.
      Wire-level audit across /v1/transactions{*} + /v1/contracts/:id{*}
      confirmed no path returns 5xx solely from ingestion lag or upstream
      archive outage. Plus path-param validators consolidated into
      `common/path.rs` (per spec literal Step 1) — `transactions` and
      `contracts` handlers now call `path::hash` / `path::strkey` instead
      of inline checks, and `contracts/handlers.rs` dropped its local
      `err()` + `is_valid_strkey` in favour of the shared
      `common::errors::*` builders. Four new error codes added to
      `errors.rs` (`invalid_hash`, `invalid_contract_id`,
      `invalid_account_id`, `invalid_sequence`). Nine new integration
      tests in `crates/api/src/tests_integration.rs`:
      `detail_invalid_hash_format_returns_400_before_db`,
      `detail_unknown_hash_returns_404_not_500`,
      `list_with_unreachable_s3_returns_200_with_degraded_memo`,
      `contract_invalid_id_returns_400_before_db`,
      `contract_invocations_invalid_id_returns_400_before_db`,
      `contract_events_invalid_id_returns_400_before_db`,
      `contract_unknown_id_returns_404_not_500`,
      `contract_interface_unknown_returns_404`,
      `out_of_u32_range_ledger_sequence_fails_conversion_safely`. 87/87
      api-crate tests pass; clippy clean. Step 5 (unknown-op `raw_xdr`)
      deferred — compile-time exhaustive match in xdr-parser plus
      light-slice "unknown" fallback cover the resilience requirement;
      literal `raw_xdr` would be dead code while the compile-time guard
      holds. Pending confirmation; backlog task spawned only if a
      mid-protocol-bump scenario materialises in practice.
  - date: '2026-04-28'
    status: completed
    who: karolkow
    note: >
      Completed. Steps 1-4 + 6 shipped (0042/0043/0046/0150 +
      `common/path.rs` + audit + 9 graceful-degradation locks in
      `tests_integration.rs`). Step 5 (`raw_xdr` for unknown ops)
      deferred — compile-time exhaustive match in `xdr-parser` plus
      light fallback `"unknown"` cover the resilience requirement; no
      backlog task spawned unless mid-protocol-bump scenario
      materialises in practice. 87/87 api-crate tests pass; clippy
      clean. AC 1-6, 8-10 satisfied; AC 7 deferred with documented
      rationale. Body realigned to ADR 0008 envelope shape and
      lowercase canonical codes. Path-param validators consolidated
      into `common/path.rs` per spec literal Step 1. No new ADRs
      required (consumes 0005, 0008, 0029).
---

# Backend: request validation, response serialization, error mapping

## Summary

Implement the cross-cutting request validation, response serialization, and error mapping layer for the API. This includes input validation extractors, response shaping rules, error-to-HTTP mapping, parse_error handling for degraded transactions, unknown operation type handling, and graceful degradation when ingestion is behind.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Active

**Current state:** Active. ~80% of original surface shipped via 0043 (validation/error helpers under `crates/api/src/common/`) and 0046 (`parse_error` field, unknown-op fallback). Step 6 (graceful-degradation audit + 3 locked-in tests) shipped on this branch 2026-04-28. Step 5 (`raw_xdr` payload on unknown ops) **deferred** pending team sync.

## Context

The API must present consistent, frontend-friendly responses while handling edge cases such as parse errors, unknown operation types, and ingestion lag. Validation, serialization, and error handling are cross-cutting concerns used by all modules.

### API Specification

**Location:** `crates/api/src/common/` — shipped under 0043 as `extractors.rs`, `filters.rs`, `errors.rs`, `cursor.rs`, `pagination.rs` (flat module files, not subdirectories as the original draft assumed). Extended on this branch with `path.rs` (path-param validators).

#### What lives in each shipped module

| File                                                               | Public API                                                                                                                                                                                                                                                                                                                                             | Used by                                                                                                                                        |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `extractors.rs`                                                    | `Pagination<P> { limit: u32, cursor: Option<P> }` — axum `FromRequestParts` extractor for `?limit=&cursor=`. Hardcoded policy: `DEFAULT_LIMIT = 20`, `MAX_LIMIT = 100`. Tolerates unknown query fields so it composes with a sibling `Query<FilterParams>` on the same handler.                                                                        | every list handler (0046 ships; 0049 / 0050 / 0047 planned)                                                                                    |
| `filters.rs`                                                       | `strkey(value, prefix, filter_key)` — RFC 4648 base32 + 56-char shape check (no CRC); `strkey_opt` for `Option<&str>`; `parse_enum::<T: FromStr>(value, filter_key, kind_hint)` + `parse_enum_opt` — both return `Result<T, Response>` for `?`-prop into `invalid_filter` envelopes                                                                    | 0046 (`source_account`, `contract_id`, `operation_type`); future 0049 / 0050 / 0051 / 0052                                                     |
| `errors.rs`                                                        | Code consts: `INVALID_CURSOR`, `INVALID_LIMIT`, `INVALID_QUERY`, `INVALID_FILTER`, `NOT_FOUND`, `DB_ERROR`. Builders: `bad_request(code, msg)`, `bad_request_with_details(code, msg, json)`, `not_found(msg)`, `internal_error(code, msg)` — all return `axum::response::Response` ready to `?`-propagate                                              | every handler that can fail                                                                                                                    |
| `cursor.rs`                                                        | Generic `encode<P: Serialize>(p) -> String` / `decode<P: DeserializeOwned>(s) -> Result<P, CursorError>` over base64url(JSON); default payload `TsIdCursor { ts, id }`. Bespoke payloads (sequence-only, id-only) define their own struct                                                                                                              | 0046 (`TsIdCursor`); future 0047 (`SequenceCursor`), 0049 / 0051 / 0052 (`IdCursor`)                                                           |
| `pagination.rs`                                                    | `finalize_page<Row>(rows, limit, key_fn)` — generic limit+1 → `(rows, PageInfo)`; `finalize_ts_id_page` convenience for `(ts, id)` ordering; `into_envelope<T>(items, page) -> Paginated<T>`; `push_ts_id_cursor_predicate` — `(ts, id) <` predicate for sqlx `QueryBuilder`                                                                           | 0046; pattern siblings to be added per ordering scheme as 0047/0049/etc land                                                                   |
| `path.rs`                                                          | `hash(value)` — 64-char hex validator (`invalid_hash`); `strkey(value, prefix, param)` — Stellar StrKey validator with prefix-aware code (`invalid_contract_id` for `'C'`, `invalid_account_id` for `'G'`); `sequence(value)` — `u32` numeric validator (`invalid_sequence`, reserved for 0047). All return `Result<_, Response>` for `?`-propagation. | 0046 (`get_transaction` hash), 0050 (`get_contract` / `get_interface` / `list_invocations` / `list_events` contract_id), 0047 / 0049 (planned) |
| `openapi/schemas.rs` (delivered by 0042 + ADR 0008, consumed here) | `ErrorEnvelope { code, message, details }`, `PageInfo { cursor, limit, has_more }`, `Paginated<T> { data, page }` — registered as OpenAPI components                                                                                                                                                                                                   | every handler, error path, and list endpoint                                                                                                   |

> **Path params (hash, account_id, contract_id, sequence)** are validated via `common::path::*` helpers at handler entry. Original draft proposed inline-per-module; refactored 2026-04-28 onto shared helpers per spec literal Step 1 ("axum extractors with validation for common parameter patterns: path params (hash, account_id, contract_id, sequence)"). Helpers are stand-alone functions rather than `FromRequestParts` impls because each path param has a different envelope code (`invalid_hash` vs `invalid_contract_id` vs `invalid_account_id` vs `invalid_sequence`) and a different `details` shape — a generic extractor would force a uniform code and lose the per-resource hint, which the spec's "descriptive and actionable" requirement (AC #10) explicitly disallows.

### Response Shaping Rules

1. **Flatten/restructure** nested data for client usability
2. **Attach human-readable labels** produced during ingestion
3. **Raw payloads only** for advanced/detail views, never in list responses
4. **Stable identifier fields** for cross-page linking (hash, account_id, contract_id, etc.)

### Error Envelope

All errors use the flat envelope shape fixed by [ADR 0008](../../2-adrs/0008_error-envelope-and-pagination-shape.md) (no outer `error` wrapper):

```json
{
  "code": "invalid_cursor",
  "message": "cursor is malformed or expired",
  "details": null
}
```

### Error-to-HTTP Mapping

Canonical codes shipped in `crates/api/src/common/errors.rs` under task 0043.

| HTTP Status | Condition                                                               | Code                                   |
| ----------- | ----------------------------------------------------------------------- | -------------------------------------- |
| 400         | `?limit=` zero / negative / non-numeric / above per-endpoint max        | `invalid_limit`                        |
| 400         | `?cursor=` failed base64 / JSON decode or wrong schema                  | `invalid_cursor`                       |
| 400         | Query string itself malformed (bad percent-encoding, duplicate keys, …) | `invalid_query`                        |
| 400         | `filter[key]=` value not interpretable (unknown enum, bad StrKey, …)    | `invalid_filter`                       |
| 400         | Path `:hash` not 64 hex chars                                           | `invalid_hash`                         |
| 400         | Path `:contract_id` not StrKey-shape (56 chars / prefix `C` / RFC 4648) | `invalid_contract_id`                  |
| 400         | Path `:account_id` not StrKey-shape (56 chars / prefix `G` / RFC 4648)  | `invalid_account_id`                   |
| 400         | Path `:sequence` not positive `u32`                                     | `invalid_sequence` (reserved for 0047) |
| 404         | Resource not found by primary key (hash, ID, …)                         | `not_found`                            |
| 500         | Unrecoverable database error (cause logged server-side, never returned) | `db_error`                             |

### parse_error Handling

Transactions with `parse_error=true` in the database:

- Remain visible in list and detail endpoints
- Non-XDR fields (hash, ledger_sequence, source_account, fee_charged, successful, created_at) served normally
- XDR-derived fields (operations, operation_tree, events) may be null
- Response includes `parse_error: true` indicator so frontend can display appropriate messaging

**Example response for parse_error transaction:**

```json
{
  "hash": "7b2a8c...",
  "ledger_sequence": 12345678,
  "source_account": "GABC...XYZ",
  "successful": true,
  "fee_charged": 100,
  "created_at": "2026-03-20T12:00:00Z",
  "operations": null,
  "operation_tree": null,
  "events": null,
  "parse_error": true
}
```

### Unknown Operation Types

When an operation type is not recognized by the current SDK version:

```json
{
  "type": "unknown",
  "raw_xdr": "AAAAAA..."
}
```

- Never hide the parent transaction because of an unknown operation
- The transaction remains fully visible with the unknown operation rendered inline

### Graceful Degradation

- All endpoints function when ingestion is behind the network tip
- No errors solely due to stale data
- Freshness is communicated via network stats (latest_ledger_sequence, latest_ledger_closed_at — frontend derives lag from the timestamp)
- Missing recent data simply means it has not been indexed yet, not an error condition

### Caching

- Validation and serialization are stateless; no caching at this layer.

### Error Handling

Input validation errors (flat envelope per ADR 0008):

```json
{
  "code": "invalid_filter",
  "message": "filter[type] is not a recognized asset type",
  "details": { "filter": "type", "received": "foo" }
}
```

Resource not found:

```json
{
  "code": "not_found",
  "message": "Transaction with hash 'abc123' not found."
}
```

## Implementation Plan

### Step 1: Input Validation Pipes — ✅ shipped via 0043

Create axum extractors with validation for common parameter patterns: path params (hash, account_id, contract_id, sequence), query params (limit, cursor), and filter params. Map validation failures to 400 responses with descriptive messages.

> Delivered: `Pagination<P>` extractor in `common/extractors.rs`; StrKey + enum filter validators in `common/filters.rs`; path-param validators in `common/path.rs` (`hash`, `strkey`, `sequence`) consumed by `transactions::get_transaction` (hash) and `contracts::{get_contract, get_interface, list_invocations, list_events}` (contract_id). Path validators emit dedicated codes (`invalid_hash`, `invalid_contract_id`, `invalid_account_id`, `invalid_sequence`) — distinct from the query-string `invalid_filter` so a client reading the `code` knows immediately whether the bad value came from URL path or query string.

### Step 2: Centralized Error Mapping — ✅ shipped via 0043 (axum idiom)

Implement a single canonical mapping from failure conditions to HTTP responses. Every error path — validation, missing resource, DB failure — must land in the same `ErrorEnvelope` shape with no hand-rolled variants.

> Delivered as the `common::errors` module (single file = single source of truth). Builders: `bad_request(code, msg)`, `bad_request_with_details(code, msg, json)`, `not_found(msg)`, `internal_error(code, msg)`. Codes: `invalid_cursor`, `invalid_limit`, `invalid_query`, `invalid_filter`, `not_found`, `db_error`. Handlers `?`-propagate `Result<T, Response>` so every error boundary funnels through these builders — cannot drift onto a hand-rolled JSON body. axum has no NestJS-style global filter type, but the _function_ (one canonical mapping; no module-local error JSON) is achieved by making `common::errors` the only source of `ErrorEnvelope` responses across the crate.

### Step 3: Response Shape Enforcement — ✅ shipped via 0042 / 0043 / 0046 / 0150

Create a axum middleware or serialization layer that applies response shaping rules: flatten nested fields, ensure stable identifiers are present, strip raw payloads from non-advanced responses.

### Step 4: parse_error Handling — ✅ shipped via 0046

Implement a serialization rule that detects `parse_error=true` on transaction records, sets XDR-derived fields to null, and includes the `parse_error` indicator in the response.

> Delivered as `parse_error: bool` on `TransactionDetailLight` (light slice, always served) plus `heavy: null` + `heavy_fields_status: "unavailable"` when the read-time XDR fetch fails (ADR 0029).

### Step 5: Unknown Operation Type Handling — ⊘ deferred (resilience covered by compile-time guard)

Implement fallback serialization for unrecognized operation types, rendering them as `{ type: 'unknown', raw_xdr: '...' }` without hiding the parent transaction.

> Decision 2026-04-28 (pending team sync confirmation): **do not build the `raw_xdr` field now.**

### Step 6: Graceful Degradation Verification — ✅ shipped on this branch

Ensure no endpoint throws errors solely because ingestion is behind. Verify that empty result sets and missing recent data are handled as normal (empty list, 404 for specific missing resource) rather than error conditions.

> Wire-level audit completed across shipped endpoints (0046 transactions, 0050 contracts). 0045 network stats and 0049 assets are still active under their own owners — picked up when they ship. Audit findings + locked-in tests below.

#### Audit findings (2026-04-27)

Read every shipped handler error branch, catalogued each `Response`-emitting site:

| Endpoint                            | Lag-related path                                                   | Mapping                                                                                                                                                                               | OK? |
| ----------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --- |
| `GET /v1/transactions`              | DB returns 0 rows for the requested filter / cursor                | 200 `{ data: [], page: { has_more: false } }` via `finalize_ts_id_page` + `into_envelope`                                                                                             | ✓   |
| `GET /v1/transactions`              | Public-archive S3 fetch fails for memo enrichment                  | `tracing::warn!` + per-row `memo = None` (`unwrap_or((None, None))` in `list_transactions`)                                                                                           | ✓   |
| `GET /v1/transactions`              | DB connection error                                                | 500 `db_error` — real fault, not a lag scenario                                                                                                                                       | ✓   |
| `GET /v1/transactions/:hash`        | Hash not in `transaction_hash_index` (e.g. ledger not yet indexed) | 404 `not_found` (`lookup_hash_index` returns `Ok(None)` → `errors::not_found`)                                                                                                        | ✓   |
| `GET /v1/transactions/:hash`        | S3 fetch fails for the parent ledger                               | 200 light slice + `heavy: null` + `heavy_fields_status: "unavailable"` (`get_transaction`)                                                                                            | ✓   |
| `GET /v1/transactions/:hash`        | Out-of-`u32`-range `ledger_sequence` on the row                    | 200 light slice + `heavy: null` + `heavy_fields_status: "unavailable"` (warn logged)                                                                                                  | ✓   |
| `GET /v1/contracts/:id`             | Contract not in `soroban_contracts`                                | 404 `not_found` (`get_contract`)                                                                                                                                                      | ✓   |
| `GET /v1/contracts/:id/interface`   | No `wasm_interface_metadata` row for the contract's `wasm_hash`    | 404 `not_found` (`get_interface`)                                                                                                                                                     | ✓   |
| `GET /v1/contracts/:id/invocations` | Public-archive fetch fails mid-page                                | Page expansion stops at the failure boundary (`expand_invocations`); cursor advances only past consecutively-expanded rows so the unexpanded tail is retried on next request — no 5xx | ✓   |
| `GET /v1/contracts/:id/events`      | Public-archive fetch fails mid-page                                | Same stop-and-retry pattern (`expand_events`)                                                                                                                                         | ✓   |

**No path returns 5xx solely because of ingestion lag or upstream archive outage.** Every degraded path either:

- Surfaces the missing-resource case as 404 with the canonical `not_found` envelope, or
- Returns 200 with the available slice and a degradation marker (`heavy_fields_status: "unavailable"` for E3 detail, partial page + cursor halt for E13 / E14 invocations / events, `null` memo / `null` heavy for list memo enrichment).

**Real DB / panic / out-of-range arithmetic are the only paths that hit 500** — they are genuine faults, not lag, and `tracing::error!` is logged before the envelope returns.

#### Locked-in tests

`crates/api/src/tests_integration.rs` — three new graceful-degradation tests added under "Graceful-degradation tests (task 0044 §6)":

| Test                                                       | Gating         | What it locks                                                                                                              |
| ---------------------------------------------------------- | -------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `detail_invalid_hash_format_returns_400_before_db`         | unconditional  | Malformed hash short-circuits to 400 `invalid_hash` before any DB / S3 call (no panic, no 500)                             |
| `detail_unknown_hash_returns_404_not_500`                  | `DATABASE_URL` | Well-formed hash with no DB row → 404 `not_found` with flat ADR 0008 envelope (no outer `error` wrapper)                   |
| `list_with_unreachable_s3_returns_200_with_degraded_memo`  | `DATABASE_URL` | List endpoint stays 200 when public-archive fetch fails; per-row memo / memo_type degrade to null without affecting status |
| `contract_invalid_id_returns_400_before_db`                | unconditional  | Malformed StrKey on `/v1/contracts/:id` → 400 `invalid_contract_id` with `details.expected_prefix=C`                       |
| `contract_invocations_invalid_id_returns_400_before_db`    | unconditional  | Same pre-DB short-circuit on the nested `/invocations` route                                                               |
| `contract_events_invalid_id_returns_400_before_db`         | unconditional  | Same pre-DB short-circuit on the nested `/events` route                                                                    |
| `contract_unknown_id_returns_404_not_500`                  | `DATABASE_URL` | Well-formed StrKey, no row → 404 `not_found` (locks the lag-scenario behaviour for contracts)                              |
| `contract_interface_unknown_returns_404`                   | `DATABASE_URL` | No `wasm_interface_metadata` row → 404 (interface-specific not_found path)                                                 |
| `out_of_u32_range_ledger_sequence_fails_conversion_safely` | unconditional  | Pure-logic guard: `u32::try_from(i64::MAX) / +1 / -1` all `Err`; handlers must read this as warn+skip, not panic           |

These complement the per-record degradation tests in 0046's S3-gated suite (`extract_e3_*`) by exercising the full handler chain end-to-end.

Test tally after this task: 87 unit + integration tests pass (`cargo test -p api --bin api`); clippy clean (`cargo clippy -p api --bin api --all-targets -- -D warnings`).

## Acceptance Criteria

- [x] Input validation extractors for all common parameter types — `common/extractors.rs` + `common/filters.rs` (0043) + `common/path.rs` (this branch, refactored from inline-per-module to shared helpers per spec literal Step 1)
- [x] Consistent flat error envelope `{ code, message, details }` (ADR 0008) on all error responses — `common/errors.rs` (0043)
- [x] 400 for validation failures, 404 for missing resources, 500 for internal errors — `bad_request*` / `not_found` / `internal_error` (0043)
- [x] Response shaping: flatten nested data, attach human-readable labels, stable identifiers — per-module DTOs (0046, 0049, 0050)
- [x] Raw payloads excluded from non-advanced responses — ADR 0029 light/heavy split via `E3Response<T>` (0150 + 0046)
- [x] parse_error transactions visible with available fields, XDR-derived fields null — `parse_error: bool` + `heavy_fields_status: "unavailable"` (0046)
- [ ] Unknown operations rendered as `{ type: 'unknown', raw_xdr: '...' }` — **deferred** (decision 2026-04-28, pending team sync). Compile-time exhaustive match in `xdr-parser` + light fallback `"unknown"` cover the resilience requirement; literal `raw_xdr` payload would be dead code while the compile-time guard holds. See Step 5.
- [x] Parent transactions never hidden due to unknown child operations — `db_operations` in `transactions/handlers.rs` falls back to `op_type: "unknown"` via `OperationType::try_from(op_type).unwrap_or_else(|_| "unknown".to_string())` instead of dropping the row
- [x] All endpoints function when ingestion is behind (no stale-data errors) — wire-level audit completed Step 6, 3 locked-in tests in `tests_integration.rs`
- [x] Error messages are descriptive and actionable — verified across `common/errors.rs` test suite + 0046/0050 envelopes

### AC ⇄ Shipped Mapping

Concrete artifact per AC line, with the verification anchor.

| #   | AC line                                           | Shipped artifact                                                                                                                                                                                                                                                     | Concrete example                                                                                                                                                                                      | Verified by                                                                                                                                                                                  |
| --- | ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| 1   | Input validation extractors                       | `Pagination<P>` (query); `filters::strkey` / `strkey_opt` / `parse_enum` / `parse_enum_opt` (filter values); path params validated inline per-module (0046 hash, 0049 asset_id, 0050 contract_id)                                                                    | `GET /v1/transactions?limit=999` → 400 `invalid_limit` with `details.max=100`; `GET /v1/transactions?filter[source_account]=BAD` → 400 `invalid_filter`                                               | `common/extractors.rs::tests` (10 cases) + `common/filters.rs::tests` (8 cases) + `tests_integration.rs` (4)                                                                                 |
| 2   | Flat error envelope `{ code, message, details }`  | `ErrorEnvelope` schema in `openapi/schemas.rs`; only constructed via `errors::*` builders                                                                                                                                                                            | `{"code":"invalid_cursor","message":"cursor is malformed or expired"}` — no outer `error` key                                                                                                         | `common/errors.rs::tests::bad_request_produces_flat_envelope` + 4 sibling tests                                                                                                              |
| 3   | 400 / 404 / 500 status mapping                    | `bad_request*` → 400, `not_found` → 404, `internal_error` → 500 (fn signatures fix the status; cannot drift)                                                                                                                                                         | DB failure → `internal_error(DB_ERROR, "database error")` → 500 `{"code":"db_error",…}`                                                                                                               | `internal_error_uses_500_and_flat_envelope` test                                                                                                                                             |
| 4   | Flatten / labels / stable IDs                     | Per-module flat `serde::Serialize` DTOs with utoipa `ToSchema`; stable IDs (`hash`, `id`, `contract_id`) at top level                                                                                                                                                | `TransactionListItem { hash, ledger_sequence, source_account, …, memo_type, memo }` — flat, with `hash` as stable cross-page anchor                                                                   | utoipa OpenAPI generation + per-module tests                                                                                                                                                 |
| 5   | Raw payloads excluded from list                   | List DTOs physically lack XDR fields; detail DTOs split via `E3Response<TransactionDetailLight>` (ADR 0029 / task 0150)                                                                                                                                              | `TransactionListItem` has no `envelope_xdr` / `result_xdr` / `operations[].details`; only present in detail's `heavy.*`                                                                               | type-level enforcement (`grep -n envelope_xdr crates/api/src/transactions/dto.rs` → 0 hits in list DTO)                                                                                      |
| 6   | parse_error tx visible with degraded heavy        | `parse_error: bool` in light slice; `heavy: null` + `heavy_fields_status: "unavailable"` on XDR fetch fail                                                                                                                                                           | parse_error tx returns `{hash, …, parse_error: true, operations: [...], heavy: null, heavy_fields_status: "unavailable"}`                                                                             | 0046 manual test against ledger 62248883 + 5 ignored S3 integration tests                                                                                                                    |
| 7   | Unknown ops `{ type: 'unknown', raw_xdr: '...' }` | **DEFERRED** (2026-04-28, pending team sync). Compile-time exhaustive match in `xdr-parser::extract_op_details` + light fallback `"unknown"` cover resilience; literal `raw_xdr` would be dead code while the compile-time guard holds (no run-time path reaches it) | currently `{ type: "unknown", contract_id: null }` only when DB discriminant ≥ 27; never observed in practice because workspace single-source `domain::OperationType` keeps API + indexer in lockstep | this task §5; spawn backlog task only if mid-protocol-bump scenario materialises                                                                                                             |
| 8   | Parent tx never hidden                            | `db_operations` in `transactions/handlers.rs` runs `OperationType::try_from(op_type).unwrap_or_else(                                                                                                                                                                 | \_                                                                                                                                                                                                    | "unknown".to_string())`per row — unknown discriminants render as`{ type: "unknown", contract_id }` inline instead of dropping the row or failing the parent                                  | tx whose `operations.type` cannot decode (e.g. stale API binary against newer indexer DB) still serialises with the parent visible and the offending op marked `"unknown"` | `domain::enums::operation_type::tests` (try_from / round-trip) + the inline fallback in `db_operations` |
| 9   | All endpoints function under ingestion lag        | Wire-level audit across `/v1/transactions{*}` + `/v1/contracts/:id{*}` confirmed no path returns 5xx solely from lag or S3 outage; missing-resource → 404 `not_found`, S3 fail → 200 + degradation marker                                                            | unknown hash → 404; S3 unreachable → list stays 200 with `memo: null`; contracts page expansion halts cursor at failure boundary instead of 500                                                       | this task §6 + `tests_integration::detail_invalid_hash_format_returns_400_before_db` + `detail_unknown_hash_returns_404_not_500` + `list_with_unreachable_s3_returns_200_with_degraded_memo` |
| 10  | Error messages descriptive + actionable           | Builders take `impl Into<String>`; call sites carry field name, allowed values, received value in `details`                                                                                                                                                          | `invalid_limit` carries `{"min":1,"max":100,"received":0}`; `invalid_filter` carries `{"filter":"source_account","received":"BAD","expected_prefix":"G"}`                                             | 4 validation tests in `tests_integration.rs` assert `details` shape                                                                                                                          |

## Notes

- This task provides shared infrastructure consumed by all feature module tasks (0045-0053).
- The parse_error and unknown operation handling are critical for explorer resilience during protocol upgrades.
- Graceful degradation is a fundamental architectural requirement, not an optional enhancement.
