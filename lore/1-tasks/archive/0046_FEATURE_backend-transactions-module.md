---
id: '0046'
title: 'Backend: Transactions module (list + detail + filters)'
type: FEATURE
status: completed
related_adr: ['0005', '0008', '0029']
related_tasks: ['0023', '0043', '0044', '0092', '0150']
tags: [layer-backend, transactions, filters, xdr]
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
  - date: 2026-04-01
    status: backlog
    who: fmazur
    note: 'Updated: event_interpretations enrichment deferred.'
  - date: 2026-04-01
    status: backlog
    who: stkrolikiewicz
    note: 'Updated: removed event_interpretations references — table removed from architecture (task 0098).'
  - date: '2026-04-23'
    status: active
    who: FilipDz
    note: 'Activated — task 0150 (stellar_archive library) completed. Implementing GET /v1/transactions + GET /v1/transactions/:hash.'
  - date: '2026-04-23'
    status: completed
    who: FilipDz
    note: >
      Both endpoints shipped per 0046 spec, aligned with ADR 0029.
      Normal and advanced views both call the public Stellar archive
      for memo / result_code / operation_tree / events / per-op
      function_name; advanced additionally returns envelope_xdr /
      result_xdr / per-op raw_parameters. Spec-literal flat response
      shape; 0150's `E3Response<T>` + `merge_e3_response` wrapper
      deleted (the nested `{light, heavy, heavy_fields_status}` shape
      did not match the spec). 19 unit tests pass (14 cargo test + 5
      ignored S3-network), clippy clean. Tested manually against
      ledger 62248883.
---

# Backend: Transactions module (list + detail + filters)

## Summary

Implement the Transactions module providing paginated transaction listing with filters and dual-mode transaction detail (normal and advanced views). This is the central activity-browsing module of the explorer API, handling the most complex response shaping including operation trees, events, and raw XDR for advanced inspection.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Context

Transactions are the primary explorer entity for activity browsing. The list endpoint supports table-style browsing with slim response types. The detail endpoint supports both normal and advanced/debugging views over the same resource, controlled by a query parameter.

### API Specification

**Location:** `crates/api/src/transactions/`

---

#### GET /v1/transactions

**Method:** GET

**Path:** `/transactions`

**Query Parameters:**

| Parameter                | Type   | Default | Description                 |
| ------------------------ | ------ | ------- | --------------------------- |
| `limit`                  | number | 20      | Items per page (max 100)    |
| `cursor`                 | string | null    | Opaque pagination cursor    |
| `filter[source_account]` | string | null    | Filter by source account ID |
| `filter[contract_id]`    | string | null    | Filter by contract ID       |
| `filter[operation_type]` | string | null    | Filter by operation type    |

**Response Shape (list):**

```json
{
  "data": [
    {
      "hash": "7b2a8c...",
      "ledger_sequence": 12345678,
      "source_account": "GABC...XYZ",
      "successful": true,
      "fee_charged": 100,
      "created_at": "2026-03-20T12:00:00Z",
      "operation_count": 3,
      "memo_type": "text",
      "memo": "payment for services"
    }
  ],
  "pagination": {
    "next_cursor": "eyJpZCI6MTIzfQ==",
    "has_more": true
  }
}
```

**List item fields (slim response type):**

| Field             | Type           | Description                              |
| ----------------- | -------------- | ---------------------------------------- |
| `hash`            | string         | Transaction hash (64-char hex)           |
| `ledger_sequence` | number         | Ledger this transaction belongs to       |
| `source_account`  | string         | Source account ID                        |
| `successful`      | boolean        | Whether transaction succeeded            |
| `fee_charged`     | number         | Fee charged in stroops                   |
| `created_at`      | string         | ISO 8601 timestamp                       |
| `operation_count` | number         | Number of operations                     |
| `memo_type`       | string         | Memo type (none, text, id, hash, return) |
| `memo`            | string or null | Memo value                               |

---

#### GET /v1/transactions/:hash

**Method:** GET

**Path:** `/transactions/:hash`

**Path Parameters:**

| Parameter | Type   | Description                    |
| --------- | ------ | ------------------------------ |
| `hash`    | string | Transaction hash (64-char hex) |

**Query Parameters:**

| Parameter | Type   | Default | Description                               |
| --------- | ------ | ------- | ----------------------------------------- |
| `view`    | string | null    | Set to `advanced` for raw/advanced fields |

**Response Shape (normal view):**

```json
{
  "hash": "7b2a8c...",
  "ledger_sequence": 12345678,
  "source_account": "GABC...XYZ",
  "successful": true,
  "fee_charged": 100,
  "result_code": null,
  "memo_type": "text",
  "memo": "payment for services",
  "created_at": "2026-03-20T12:00:00Z",
  "operations": [
    {
      "type": "invoke_host_function",
      "contract_id": "CCAB...DEF",
      "function_name": "swap"
    }
  ],
  "operation_tree": [],
  "events": [
    {
      "event_type": "contract",
      "topics": [],
      "data": {}
    }
  ],
  "parse_error": false
}
```

**Response Shape (advanced view, `?view=advanced`):**

```json
{
  "hash": "7b2a8c...",
  "ledger_sequence": 12345678,
  "source_account": "GABC...XYZ",
  "successful": true,
  "fee_charged": 100,
  "result_code": null,
  "memo_type": "text",
  "memo": "payment for services",
  "created_at": "2026-03-20T12:00:00Z",
  "operations": [
    {
      "type": "invoke_host_function",
      "contract_id": "CCAB...DEF",
      "function_name": "swap",
      "raw_parameters": {},
      "raw_event_payloads": []
    }
  ],
  "operation_tree": [],
  "events": [],
  "envelope_xdr": "AAAAAA...",
  "result_xdr": "AAAAAA...",
  "parse_error": false
}
```

**Detail fields:**

| Field             | Type           | Normal | Advanced | Description                   |
| ----------------- | -------------- | ------ | -------- | ----------------------------- |
| `hash`            | string         | yes    | yes      | Transaction hash              |
| `ledger_sequence` | number         | yes    | yes      | Ledger sequence               |
| `source_account`  | string         | yes    | yes      | Source account                |
| `successful`      | boolean        | yes    | yes      | Success status                |
| `fee_charged`     | number         | yes    | yes      | Fee in stroops                |
| `result_code`     | string or null | yes    | yes      | Result code for failed txs    |
| `memo_type`       | string         | yes    | yes      | Memo type                     |
| `memo`            | string or null | yes    | yes      | Memo value                    |
| `created_at`      | string         | yes    | yes      | ISO timestamp                 |
| `operations`      | array          | yes    | yes      | Decoded/normalized operations |
| `operation_tree`  | array          | yes    | yes      | Decoded invocation hierarchy  |
| `events`          | array          | yes    | yes      | Events                        |
| `envelope_xdr`    | string         | no     | yes      | Raw envelope XDR              |
| `result_xdr`      | string         | no     | yes      | Raw result XDR                |
| `parse_error`     | boolean        | yes    | yes      | Whether parse error occurred  |

**Important:** `result_meta_xdr` is NOT returned to the frontend. It is used server-side only for decode/validation. The `operation_tree` (decoded from `result_meta_xdr` at ingestion) is returned instead.

### Behavioral Requirements

- List responses optimized for table-style browsing (slim response types)
- Detail supports both human-readable and advanced views via `?view=advanced`
- Same endpoint, same resource -- two representations
- Filters applied at DB query level before pagination
- `result_code` included for failed transactions
- parse_error transactions visible with available fields; XDR-derived fields may be null
- Unknown operations rendered as `{ type: 'unknown', raw_xdr: '...' }`

### Caching

| Endpoint                           | TTL   | Notes                                          |
| ---------------------------------- | ----- | ---------------------------------------------- |
| `GET /transactions` (list)         | 5-15s | Short TTL, frequently changing                 |
| `GET /transactions/:hash` (detail) | 300s+ | Long TTL, finalized transactions are immutable |

### Error Handling

- 400: Invalid filter values, invalid hash format, invalid view param
- 404: Transaction hash not found
- 500: Database errors

## Implementation Plan

### Step 1: Route + handler setup

Create `crates/api/src/transactions/` with module, controller, service, and request/response types (ToSchema).

### Step 2: List Endpoint

Implement `GET /transactions` with cursor pagination, filter parsing, and slim response type response.

### Step 3: Detail Endpoint (Normal View)

Implement `GET /transactions/:hash` returning full detail with operations, operation_tree, events, and result_code.

### Step 4: Advanced View

Add `?view=advanced` support to the detail endpoint, including envelope_xdr, result_xdr, raw parameters, and raw event payloads.

### Step 5: parse_error and Unknown Operation Handling

Ensure parse_error transactions are visible. Render unknown operation types as `{ type: 'unknown', raw_xdr: '...' }`.

### Step 6: Filter Implementation

Implement source_account, contract_id, and operation_type filters at the DB query level.

## Acceptance Criteria

- [x] `GET /v1/transactions` returns paginated list with slim response types
- [x] `GET /v1/transactions/:hash` returns flat detail JSON in normal view (per spec)
- [x] `GET /v1/transactions/:hash?view=advanced` adds envelope_xdr, result_xdr, per-op raw_parameters
- [x] result_meta_xdr never returned to frontend (also dropped from internal `E3HeavyFields`)
- [x] operation_tree returned in BOTH views (sourced from XDR/S3, not DB — see Design Decisions §5; null on fetch failure)
- [x] Events returned with full topics + data in BOTH views (sourced from XDR/S3, not `soroban_events` — see Design Decisions §5; empty on fetch failure)
- [x] result_code present in BOTH views (null when fetch failed or `parse_error == true`)
- [x] filter[source_account], filter[contract_id], filter[operation_type] work and combine
- [x] parse_error transactions visible with null XDR-derived fields
- [x] Standard pagination envelope on list endpoint (ADR 0008)
- [x] Appropriate error responses (400, 404, 500) using `ErrorEnvelope` (ADR 0008)
- [ ] Unknown operations rendered as `{ type: 'unknown', raw_xdr: '...' }` — currently `{ type: 'unknown' }` only (deferred, see Future Work)
- [ ] Per-op `raw_event_payloads` (advanced view) — not implemented (deferred, see Future Work)

## Implementation Notes

**New files**

- `crates/api/src/state.rs` — `AppState { db: PgPool, fetcher: StellarArchiveFetcher }`
- `crates/api/src/transactions/mod.rs` — `OpenApiRouter<AppState>` mounted under `/v1`
- `crates/api/src/transactions/cursor.rs` — base64url(JSON) cursor encode/decode + 3 unit tests
- `crates/api/src/transactions/dto.rs` — `ListParams`, `DetailParams`, `TransactionListItem`, `TransactionDetailLight`, `OperationItem`, `EventItem`
- `crates/api/src/transactions/queries.rs` — dynamic `QueryBuilder` list query, hash-index lookup, detail + operations fetch
- `crates/api/src/transactions/handlers.rs` — `list_transactions`, `get_transaction`

**Modified files**

- `crates/api/src/main.rs` — async `main` builds real `PgPool` + unsigned S3 client; `app(&config, state)` takes `AppState`
- `crates/api/src/stellar_archive/dto.rs` — added `result_code`, `operation_tree` to `E3HeavyFields`; removed `result_meta_xdr` and `XdrInvocationDto` (never surfaced); deleted `E3Response<T>` (Option A — flat shape per spec)
- `crates/api/src/stellar_archive/extractors.rs` — populate `result_code` + `operation_tree` from `InvocationResult`
- `crates/api/src/stellar_archive/merge.rs` — deleted `merge_e3_response` (consumer removed); kept `merge_e14_*` for future task 0050
- `crates/api/src/stellar_archive/mod.rs` — deleted `merge_e3_stellar_heavy_with_fake_db_light` integration test (function it tested no longer exists)
- `crates/api/src/openapi/mod.rs` — registered transactions schemas; dropped now-internal `E3Response` / `E3HeavyFields` / `SignatureDto` / `XdrEventDto` / `HeavyFieldsStatus` from public components
- `crates/api/Cargo.toml` — added `base64`, `chrono`, `hex` workspace deps

**Test results:** 19 tests total — 14 pass on `cargo test -p api`, 5 ignored (require network access to `aws-public-blockchain`). All 5 ignored tests pass when run with `--ignored` against real S3. `cargo clippy --all-targets -- -D warnings` clean.

**Tested manually** against ledger 62248883: confirmed `result_code`, `memo`, `events` (full topics + data), `operation_tree`, per-op `function_name` in normal view; `envelope_xdr`/`result_xdr`/per-op `raw_parameters` additionally in advanced view; all filters; all error cases.

## Issues Encountered

- **`utoipa-axum` nest auto-prefixes path annotations.** Handler `#[utoipa::path]` annotations must use `/transactions` (not `/v1/transactions`) because `nest("/v1", ...)` adds the prefix automatically. Using `/v1/transactions` produced `/v1/v1/transactions` in the spec.
- **S3 client used `load_defaults()` (default credential chain) initially.** The chain timed out resolving local credentials. Public archive `aws-public-blockchain` requires no auth — switched to `.no_credentials().region("us-east-2").timeout_config(...)` matching the pattern in `stellar_archive` integration tests.
- **Operations table requires `ledger_sequence`, not just `transaction_id + created_at`.** Surfaced during manual test data inserts; not in original plan. Adjusted seed scripts.
- **Hash constraint is 32 bytes (64 hex chars).** Off-by-one hex literals failed the constraint; confirmed with `len('ab' * 32) == 64`.

## Design Decisions

### From Plan

1. **ADR 0029 light + heavy split.** DB stores indexed light columns; XDR (S3) provides heavy fields (memo, result_code, events, operation_tree, raw blobs). `StellarArchiveFetcher` from task 0150 is the read path. Graceful degradation: heavy unavailable → fields null.
2. **Cursor encodes `(created_at, id)`.** Both fields are needed so the partitioned `transactions` table can prune by `created_at` on the continuation query. Encoded as `base64url(JSON)`.
3. **Dynamic `QueryBuilder` for list filters.** All three filters can combine; `DISTINCT` is added when an operations join is needed to avoid duplicate rows.
4. **Hash-index lookup before detail fetch.** Unpartitioned `transaction_hash_index` gives a fast hash → `(ledger_sequence, created_at)` map so the partitioned query can prune.

### Emerged

5. **`operation_tree` and `events` come from XDR (S3), not DB.** The original spec said "operation_tree pre-computed at ingestion, read from DB" and "events from soroban_events table". In practice: `soroban_invocations` stores flat rows (no serialized tree column) and `soroban_events` stores only `topic0` for indexing (no full topics array or data JSON). Full payloads only exist in XDR. The S3 read path is the only structurally possible source.
6. **`result_code` and `operation_tree` added to `E3HeavyFields`** (task 0150 struct). `result_code` was already on `ExtractedTransaction` in `xdr_parser` but not surfaced; `operation_tree` required capturing `InvocationResult.operation_tree` (nested JSON) alongside the flat list during invocation extraction.
7. **S3 client configured for anonymous access (`.no_credentials()`).** Production Lambda also accesses a public bucket — IAM credentials are irrelevant. Eliminates credential-resolution latency on Lambda cold starts.
8. **Spec-literal flat response shape; `E3Response<T>` deleted.** An intermediate state had `?view=advanced` gating the S3 call (DB-only normal view) for performance. After re-reading ADR 0029 that deviation was reverted: 0029 explicitly accepts per-request S3 latency as the architecture's price (caching is deferred until measured), and the 0046 spec table lists memo / result*code / operation_tree / events as present in BOTH views. So both views call S3 unconditionally. 0150's `E3Response<T>` wrapper produced nested `{light, heavy, heavy_fields_status}` which doesn't match the flat spec — deleted, along with `merge_e3_response` (its only consumer). 0150's E14 equivalents (`E14EventResponse`, `merge_e14*\*`) preserved for task 0050.

## Future Work

- **Unknown ops `raw_xdr` field.** Spec says `{ type: 'unknown', raw_xdr: '...' }` — currently returns `{ type: 'unknown' }` only. Requires threading raw XDR bytes for unrecognized operation types through the extraction pipeline.
- **Per-op `raw_event_payloads`.** Spec advanced-view example shows `raw_event_payloads: []` per operation. Would require correlating each operation's emitted events back to its op slot during extraction.
- **`Cache-Control` headers.** Spec specifies TTLs (5–15 s for list, 300 s+ for detail). Not yet implemented.
- **Read-path cache for E3 / E14.** ADR 0029 §"Negative consequences" warns p95 will be measurably higher than DB-only endpoints (S3 GET ~20–100 ms + zstd ~5–20 ms + XDR deserialize ~10–50 ms). Deliberately deferred per ADR 0029 §"Decision pkt 6" until measured load proves it necessary — spawn a dedicated cache task at that point.
- **Events from `soroban_events` DB table as fallback.** Once the indexer populates full event topics + data in the DB, consider serving events from DB (always available) with XDR as enrichment.
