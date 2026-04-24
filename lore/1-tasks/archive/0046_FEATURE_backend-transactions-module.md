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
  - date: '2026-04-23'
    status: completed
    who: FilipDz
    note: >
      A2 re-alignment after merging task 0157 (ADR 0033) from develop.
      Switched `GET /v1/transactions/:hash` to use 0150's
      `E3Response<TransactionDetailLight>` wrapper via
      `merge_e3_response`. Response shape is now `{light flattened,
      heavy: {...}, heavy_fields_status: ok|unavailable}` — replaces
      the earlier flat shape. `TransactionDetailLight` trimmed to
      DB-only fields; all XDR-sourced fields (memo, result_code,
      signatures, events, per-op decoded details, envelope/result XDR,
      operation_tree) live in `heavy`. Dropped `view=advanced` query
      param — wrapper always carries the full heavy payload when
      available. Rationale: consumes 0150's wrapper as designed (the
      wrapper was authored alongside 0150 — `git blame 50817b3` —
      specifically so the first real handler could merge a DB
      tx-light slice with the XDR heavy block). Mazur's task 0157
      preserved it on develop without modification when refactoring
      the events side. The wrapper gives the front-end an explicit
      `heavy_fields_status` signal instead of inferring from null
      fields. ADR 0033 doesn't dictate E3 shape but its principle
      (handler emits canonical wrapper when merge applies) is honored.
      15 cargo tests + 5 ignored S3 tests pass, clippy clean.
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
  "page": {
    "cursor": "eyJ0cyI6IjIwMjYtMDMtMjBUMTI6MDA6MDBaIiwiaWQiOjEyM30=",
    "limit": 20,
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

**Query Parameters:** none. (Per A2 re-alignment — see Design Decisions §9 — the wrapper always carries the full heavy payload when available; the original `view=advanced` toggle is no longer meaningful.)

**Response Shape (`E3Response<TransactionDetailLight>` from task 0150):**

```json
{
  "hash": "7b2a8c...",
  "ledger_sequence": 12345678,
  "source_account": "GABC...XYZ",
  "successful": true,
  "fee_charged": 100,
  "created_at": "2026-03-20T12:00:00Z",
  "parse_error": false,
  "operations": [
    { "type": "INVOKE_HOST_FUNCTION", "contract_id": "CCAB...DEF" }
  ],
  "heavy": {
    "memo_type": "text",
    "memo": "payment for services",
    "signatures": [{ "hint": "abcd1234", "signature": "..." }],
    "fee_bump_source": null,
    "envelope_xdr": "AAAAAA...",
    "result_xdr": "AAAAAA...",
    "diagnostic_events": [],
    "contract_events": [
      { "event_type": "contract", "contract_id": "CCAB...DEF",
        "topics": [...], "data": {...}, "event_index": 0 }
    ],
    "operations": [
      { "op_type": "INVOKE_HOST_FUNCTION", "application_order": 0,
        "details": { "functionName": "swap", "args": [...] } }
    ],
    "result_code": "txSuccess",
    "operation_tree": { "calls": [...] }
  },
  "heavy_fields_status": "ok"
}
```

When the public-archive fetch fails, `heavy: null` and `heavy_fields_status: "unavailable"` — light slice still returned.

**Detail fields (light vs heavy):**

| Field                            | Block | Source | Description                                                                                                                                                            |
| -------------------------------- | ----- | ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `hash`                           | light | DB     | Transaction hash (64-char hex)                                                                                                                                         |
| `ledger_sequence`                | light | DB     | Ledger sequence                                                                                                                                                        |
| `source_account`                 | light | DB     | Source account                                                                                                                                                         |
| `successful`                     | light | DB     | Success status                                                                                                                                                         |
| `fee_charged`                    | light | DB     | Fee in stroops                                                                                                                                                         |
| `created_at`                     | light | DB     | ISO timestamp                                                                                                                                                          |
| `parse_error`                    | light | DB     | XDR parse error flag                                                                                                                                                   |
| `operations[].type`              | light | DB     | Operation type tag in SCREAMING_SNAKE_CASE (e.g. `INVOKE_HOST_FUNCTION`, `PAYMENT`) — canonical Horizon convention; matches `filter[operation_type]=…` accepted values |
| `operations[].contract_id`       | light | DB     | Contract StrKey when applicable                                                                                                                                        |
| `heavy.memo_type` / `heavy.memo` | heavy | XDR    | Memo                                                                                                                                                                   |
| `heavy.result_code`              | heavy | XDR    | Tx result code (e.g. `txSuccess`)                                                                                                                                      |
| `heavy.signatures`               | heavy | XDR    | Envelope signatures                                                                                                                                                    |
| `heavy.fee_bump_source`          | heavy | XDR    | Fee-bump StrKey if any                                                                                                                                                 |
| `heavy.envelope_xdr`             | heavy | XDR    | Base64 envelope                                                                                                                                                        |
| `heavy.result_xdr`               | heavy | XDR    | Base64 result                                                                                                                                                          |
| `heavy.diagnostic_events`        | heavy | XDR    | Diagnostic events                                                                                                                                                      |
| `heavy.contract_events`          | heavy | XDR    | Contract + system events with full topics + data                                                                                                                       |
| `heavy.operations[]`             | heavy | XDR    | Per-op decoded JSON details                                                                                                                                            |
| `heavy.operation_tree`           | heavy | XDR    | Nested Soroban invocation tree                                                                                                                                         |
| `heavy_fields_status`            | meta  | —      | `"ok"` or `"unavailable"`                                                                                                                                              |

`result_meta_xdr` is **never** returned to the frontend (extraction-time only — `operation_tree` is its derived form).

> **A2 re-alignment note:** earlier intermediate states of this spec showed a flat shape with `envelope_xdr` / `result_xdr` at the top level (advanced view) and a separate `?view=advanced` toggle. After merging task 0157 / ADR 0033 from develop, the endpoint adopted 0150's `E3Response<T>` wrapper instead — see Design Decisions §9.

### Behavioral Requirements

- List responses optimized for table-style browsing (slim response types)
- Detail returns the full wrapped `E3Response<TransactionDetailLight>` per ADR 0029 + 0150 design
- Filters applied at DB query level before pagination
- `result_code` present in `heavy.result_code` for every transaction (null when fetch fails or when `parse_error == true`)
- `parse_error` transactions visible with `heavy: null` and `heavy_fields_status: "unavailable"` if the XDR fetch failed
- Unknown operations rendered as `{ type: 'unknown', raw_xdr: '...' }` (deferred — see Future Work)

### Caching

| Endpoint                           | TTL   | Notes                                          |
| ---------------------------------- | ----- | ---------------------------------------------- |
| `GET /transactions` (list)         | 5-15s | Short TTL, frequently changing                 |
| `GET /transactions/:hash` (detail) | 300s+ | Long TTL, finalized transactions are immutable |

### Error Handling

- 400: Invalid filter values, invalid hash format
- 404: Transaction hash not found
- 500: Database errors

## Implementation Plan

### Step 1: Route + handler setup

Create `crates/api/src/transactions/` with module, controller, service, and request/response types (ToSchema).

### Step 2: List Endpoint

Implement `GET /transactions` with cursor pagination, filter parsing, and slim response type response.

### Step 3: Detail Endpoint

Implement `GET /transactions/:hash` returning the full wrapped `E3Response<TransactionDetailLight>` (light slice flattened + `heavy:` block + `heavy_fields_status`). All XDR-sourced fields — memo, result_code, signatures, events, per-op decoded details, envelope_xdr, result_xdr, operation_tree — live in `heavy`.

> **Note:** original spec drafted Step 3 as "normal view" + Step 4 as `?view=advanced` (flat shape with raw_parameters / raw_event_payloads top-level). After A2 re-alignment (see Design Decisions §9) the wrapper subsumes both — there is no `?view=advanced` query param. Step 4 is folded into Step 3.

### Step 4: ~~Advanced View~~ — superseded by A2 wrapper (see Step 3)

### Step 5: parse_error and Unknown Operation Handling

Ensure parse_error transactions are visible. Render unknown operation types as `{ type: 'unknown', raw_xdr: '...' }`.

### Step 6: Filter Implementation

Implement source_account, contract_id, and operation_type filters at the DB query level.

## Acceptance Criteria

- [x] `GET /v1/transactions` returns paginated list with slim response types
- [x] `GET /v1/transactions/:hash` returns wrapped `E3Response<TransactionDetailLight>` (light slice flattened + `heavy:` block + `heavy_fields_status`) per A2 — see Design Decisions §9
- [x] `envelope_xdr`, `result_xdr`, per-op decoded `details` available inside `heavy.*` (no separate `view=advanced` query param needed — wrapper always carries full heavy when available)
- [x] `result_meta_xdr` never returned to frontend (also dropped from internal `E3HeavyFields`)
- [x] `heavy.operation_tree` populated when fetch succeeds (sourced from XDR/S3, not DB — see Design Decisions §5; null on fetch failure)
- [x] `heavy.contract_events` carries full topics + data (sourced from XDR/S3, not `soroban_events` — see Design Decisions §5)
- [x] `heavy.result_code` present (null when fetch failed or `parse_error == true`)
- [x] filter[source_account], filter[contract_id], filter[operation_type] work and combine
- [x] `parse_error` transactions visible — light slice always returned; `heavy_fields_status: "unavailable"` when XDR fetch fails
- [x] Standard pagination envelope on list endpoint (ADR 0008)
- [x] Appropriate error responses (400, 404, 500) using `ErrorEnvelope` (ADR 0008)
- [ ] Unknown operations rendered as `{ type: 'unknown', raw_xdr: '...' }` — currently `{ type: 'unknown' }` only (deferred, see Future Work)
- ~~Per-op `raw_event_payloads`~~ — **superseded by ADR 0033.** Events are surfaced globally in `heavy.contract_events` at transaction level, not per-operation. ADR 0033 collapses event detail to read-time XDR fetch from S3 with no per-op split; the spec's original per-op shape is no longer applicable. Frontend can correlate events to ops via topic inspection if needed.

## Implementation Notes

**New files**

- `crates/api/src/state.rs` — `AppState { db: PgPool, fetcher: StellarArchiveFetcher }`
- `crates/api/src/transactions/mod.rs` — `OpenApiRouter<AppState>` mounted under `/v1`
- `crates/api/src/transactions/cursor.rs` — base64url(JSON) cursor encode/decode + 3 unit tests
- `crates/api/src/transactions/dto.rs` — `ListParams`, `TransactionListItem`, `TransactionDetailLight` (DB-only after A2), `OperationItem` (type + contract_id only)
- `crates/api/src/transactions/queries.rs` — dynamic `QueryBuilder` list query, hash-index lookup, detail + operations fetch
- `crates/api/src/transactions/handlers.rs` — `list_transactions`, `get_transaction` (uses `merge_e3_response` after A2)

**Modified files**

- `crates/api/src/main.rs` — async `main` builds real `PgPool` + unsigned S3 client; `app(&config, state)` takes `AppState`
- `crates/api/src/stellar_archive/dto.rs` — added `result_code`, `operation_tree` to `E3HeavyFields`; removed `result_meta_xdr` and `XdrInvocationDto` (never surfaced). After A2 / merging 0157: kept `E3Response<T>` (originally added in 0150) and dropped E14 wrappers per ADR 0033.
- `crates/api/src/stellar_archive/extractors.rs` — populate `result_code` + `operation_tree` from `InvocationResult`
- `crates/api/src/stellar_archive/merge.rs` — kept `merge_e3_response` (originally added in 0150, now used after A2); E14 helpers removed by 0157.
- `crates/api/src/openapi/mod.rs` — registered `E3Response<TransactionDetailLight>` + helper schemas (`E3HeavyFields`, `SignatureDto`, `XdrEventDto`, `XdrOperationDto`, `HeavyFieldsStatus`)
- `crates/api/Cargo.toml` — added `base64`, `chrono`, `hex` workspace deps

**Test results:** 20 tests total — 15 pass on `cargo test -p api`, 5 ignored (require network access to `aws-public-blockchain`). All 5 ignored tests pass when run with `--ignored` against real S3. `cargo clippy --all-targets -- -D warnings` clean.

**Tested manually** against ledger 62248883 (pre-A2 state): confirmed `result_code`, `memo`, `events` (full topics + data), `operation_tree`, per-op `function_name`, `envelope_xdr`/`result_xdr`, all filters, all error cases. Post-A2 wrap re-tested via the 5 ignored S3 integration tests covering the underlying `extract_e3_heavy` + `merge_e3_response` path.

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
8. **Spec-literal flat response shape; `E3Response<T>` deleted.** An intermediate state had `?view=advanced` gating the S3 call (DB-only normal view) for performance. After re-reading ADR 0029 that deviation was reverted: 0029 explicitly accepts per-request S3 latency as the architecture's price (caching is deferred until measured), and the 0046 spec table lists memo / result_code / operation_tree / events as present in BOTH views. So both views call S3 unconditionally. 0150's `E3Response<T>` wrapper produced nested `{light, heavy, heavy_fields_status}` which doesn't match the flat spec — deleted, along with `merge_e3_response` (its only consumer). **Superseded by §9.**

9. **A2 re-alignment: adopt 0150's `E3Response<T>` wrapper after merging task 0157 (ADR 0033) from develop.** `E3Response<T>` and `merge_e3_response` were authored alongside the rest of 0150 (`stellar_archive` library) — `git blame 50817b3` — specifically so the first real handler (= this task) could merge a DB tx-light slice with the XDR heavy block. Mazur's 0157 refactor on develop preserved them unchanged when stripping the E14 wrappers. Going with the flat shape (§8) would have left these types as dead code in 0150 — diminishing its design value. After re-checking ADR 0033 and the original 0150 spec ("ready to merge with DB light row") we adopted the wrapper:

   - `TransactionDetailLight` trimmed to DB-only (hash, ledger_sequence, source_account, successful, fee_charged, created_at, parse_error, operations[type+contract_id])
   - `OperationItem` trimmed to `type` + `contract_id` only — XDR-decoded per-op details live in `heavy.operations[]`
   - Handler returns `Json(merge_e3_response(light, heavy))`
   - `?view=advanced` query param removed — wrapper always carries the full heavy payload when fetch succeeds
   - `EventItem` DTO removed — events live in `heavy.contract_events` as `XdrEventDto`
   - OpenAPI components register `E3Response<TransactionDetailLight>` plus helpers (`E3HeavyFields`, `SignatureDto`, `XdrEventDto`, `XdrOperationDto`, `HeavyFieldsStatus`)
   - Frontend gets explicit `heavy_fields_status` ("ok" / "unavailable") instead of inferring from null fields

   ADR 0033 doesn't dictate E3 response shape (it focuses on event sourcing — appearance index + S3) but its spirit is honored: the handler consumes the canonical 0150 wrapper. Trade-off: the original spec example (drafted before 0150 existed) showed a flat shape; the response shape table in this doc has been updated to reflect the wrapper.

10. **E3 deliberately bypasses `soroban_events_appearances` query for events.** ADR 0033 §Decision pkt 5 prescribes "DB appearances + S3 full expansion" for E3, E10, E14. For E3 specifically, the appearance index is redundant: we already know the parent ledger from `transaction_hash_index → transactions.ledger_sequence` (1 row, exactly 1 ledger), and we're already fetching that ledger's XDR for memo / signatures / envelope_xdr — events fall out of the same fetch for free. A query against `soroban_events_appearances WHERE transaction_id = ?` would only confirm "events exist" without changing the fetch path. The appearance index is load-bearing for **contract-side pagination** (E10 / E14 — task 0050), not for single-tx fetch (E3). The intent of ADR 0033 — "all parsed event detail defers to read-time XDR fetch with no DB fallback" — is fully satisfied: `heavy.contract_events` comes from the parent ledger XDR, not from any DB column. **No outstanding ADR 0033 work for this task.**

## Future Work

- **Unknown ops `raw_xdr` field.** Spec says `{ type: 'unknown', raw_xdr: '...' }` — currently returns `{ type: 'unknown' }` only. Requires threading raw XDR bytes for unrecognized operation types through the extraction pipeline.
- **Per-op `raw_event_payloads`.** Spec advanced-view example shows `raw_event_payloads: []` per operation. Would require correlating each operation's emitted events back to its op slot during extraction.
- **`Cache-Control` headers.** Spec specifies TTLs (5–15 s for list, 300 s+ for detail). Not yet implemented.
- **Read-path cache for E3 / E14.** ADR 0029 §"Negative consequences" warns p95 will be measurably higher than DB-only endpoints (S3 GET ~20–100 ms + zstd ~5–20 ms + XDR deserialize ~10–50 ms). Deliberately deferred per ADR 0029 §"Decision pkt 6" until measured load proves it necessary — spawn a dedicated cache task at that point.
- **Events from `soroban_events` DB table as fallback.** Once the indexer populates full event topics + data in the DB, consider serving events from DB (always available) with XDR as enrichment.
