---
id: '0047'
title: 'Backend: Ledgers module (list + detail + linked transactions)'
type: FEATURE
status: completed
related_adr: ['0005', '0008', '0029', '0037']
related_tasks: ['0023', '0043', '0046', '0092', '0167']
tags: [layer-backend, ledgers]
milestone: 2
links:
  - 'docs/architecture/database-schema/endpoint-queries/04_get_ledgers_list.sql'
  - 'docs/architecture/database-schema/endpoint-queries/05_get_ledgers_by_sequence.sql'
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: axum → Rust (axum + utoipa + sqlx)'
  - date: 2026-04-27
    status: active
    who: karolkow
    note: 'Activated task'
  - date: 2026-04-28
    status: active
    who: karolkow
    note: >
      Spec audit vs current project state. Updates: (1) response envelope
      `pagination` → `page` per ADR 0008; (2) error envelope flat per ADR
      0008; (3) embedded transactions[] in detail are DB-only — same DB
      pattern as `GET /v1/transactions` list, no archive XDR fetch. The
      `TransactionListItem` DTO carries the seven structural fields only
      (hash, ledger_sequence, source_account, successful, fee_charged,
      created_at, operation_count); memo and other heavy fields live on
      the transaction detail endpoint's E3 heavy block, not on list
      rows. The original 0167 plan to source embedded txs from a
      per-ledger S3 JSON blob was abandoned by ADR 0029, and the brief
      intermediate plan to keep a memo overlay via `extract_e3_memo` was
      dropped — list endpoints stay archive-free; (4) head-ledger
      detection via `next_sequence is None` from the LATERAL prev/next
      lookup (no extra query, no NetworkStats coupling); (5)
      Cache-Control emitted per-handler (network/handlers.rs pattern);
      (6) cursor + pagination helper conventions from task 0043
      (project-default `TsIdCursor` reused for both lists — embedded txs
      key `(created_at, id)`, ledgers key `(closed_at, sequence)` —
      cursor wire format is opaque per ADR 0008 so the field-name reuse
      is fine). Canonical SQL refs (04, 05) added to links; SQL 05
      simplified to pure DB-only two-statement form.
  - date: 2026-04-29
    status: completed
    who: karolkow
    note: >
      Implemented and shipped. 4 files in `crates/api/src/ledgers/`
      (mod 28 / dto 61 / queries 210 / handlers 201 = 500 lines).
      `transactions::list_transactions` simplified — memo enrichment
      loop dropped (`memo_type` / `memo` removed from
      `TransactionListItem`; live only in tx detail E3 heavy block).
      `extract_e3_memo` deleted (no callers after slimming).
      Canonical SQL 05 + spec 0047 + DTO comments aligned with DB-only
      contract. LATERAL prev/next switched from `closed_at` to
      `sequence` for index-only scan on PK. 90 tests pass (+8 vs
      pre-task: 1 OpenAPI smoke + 7 integration covering validation,
      cursor traversal, 404, head-vs-closed Cache-Control, prev/next
      NULL handling). cargo check + clippy + tests green.
---

# Backend: Ledgers module (list + detail + linked transactions)

## Summary

Implement the Ledgers module providing paginated ledger listing and ledger detail with linked transactions. Closed ledgers are immutable and should be served with long-TTL cache headers. This is a straightforward historical/browsing module.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in `crates/api/`.
>
> **Canonical SQL** (hand-tuned, source of truth for read shape):
>
> - List: [`04_get_ledgers_list.sql`](../../../docs/architecture/database-schema/endpoint-queries/04_get_ledgers_list.sql)
> - Detail: [`05_get_ledgers_by_sequence.sql`](../../../docs/architecture/database-schema/endpoint-queries/05_get_ledgers_by_sequence.sql)
>
> Where the SQL header conflicts with this spec on field names/types, the SQL wins (it was written against ADR 0037 schema). Behavioral / contract decisions still live here.

## Status: Completed

**Current state:** Spec refreshed 2026-04-28 against ADRs 0008 / 0029 / 0037 and sibling modules (0045 network, 0046 transactions, 0049 assets). Implementation landed on `feat/0047_backend-ledgers-module` (DB-only list + detail with embedded paginated transactions, head detection via `next_sequence is None`, integration tests gated on `DATABASE_URL`). Hard deps satisfied: 0023 (bootstrap), 0043 (pagination — `TsIdCursor` codec + `Pagination<P>` extractor + `finalize_ts_id_page`), 0046 (transactions module — `TransactionListItem` DTO reused, internal `LedgerTxRow` structure mirrors `TxListRow`).

## Context

Ledgers are the backbone of the explorer timeline. The list endpoint supports browsing recent ledgers in reverse sequence order. The detail endpoint includes linked transactions for a specific ledger. Since closed ledgers are immutable, aggressive caching is appropriate.

### API Specification

**Location:** `crates/api/src/ledgers/`

---

#### GET /v1/ledgers

**Method:** GET

**Path:** `/ledgers`

**Query Parameters:**

| Parameter | Type   | Default | Description              |
| --------- | ------ | ------- | ------------------------ |
| `limit`   | number | 20      | Items per page (max 100) |
| `cursor`  | string | null    | Opaque pagination cursor |

**Default ordering:** `(closed_at DESC, sequence DESC)` — compound for total cursor ordering even in the (very rare) tie case. Index used: `idx_ledgers_closed_at`.

**Response Shape (list):**

Envelope per ADR 0008 (`Paginated<T>` from `crates/api/src/openapi/schemas.rs`): nested `page` object with `cursor` / `limit` / `has_more`.

```json
{
  "data": [
    {
      "sequence": 12345678,
      "hash": "abcdef...",
      "closed_at": "2026-03-20T12:00:00Z",
      "protocol_version": 21,
      "transaction_count": 150,
      "base_fee": 100
    }
  ],
  "page": {
    "cursor": "eyJ0cyI6IjIwMjYtMDMtMjBUMTI6MDA6MDBaIiwiaWQiOjEyMzQ1Njc3fQ",
    "limit": 20,
    "has_more": true
  }
}
```

Cursor type: reuse the project-default `crate::common::cursor::TsIdCursor`. Mapping: `cursor.ts` carries the row's `closed_at`, `cursor.id` carries the row's `sequence`. Cursor wire format is opaque (base64url JSON) per ADR 0008, so the `ts` / `id` field names in the payload are an internal detail — clients never see or construct them by hand.

The keyset predicate is inlined in canonical SQL 04: `WHERE $cursor_closed_at IS NULL OR (l.closed_at, l.sequence) < ($cursor_closed_at, $cursor_sequence)`. No new helper needed in `common/pagination.rs` — `finalize_ts_id_page` covers the encode side and `cursor::encode`/`decode` (via the `Pagination<TsIdCursor>` extractor) cover the decode side.

---

#### GET /v1/ledgers/:sequence

**Method:** GET

**Path:** `/ledgers/:sequence`

**Path Parameters:**

| Parameter  | Type   | Description            |
| ---------- | ------ | ---------------------- |
| `sequence` | number | Ledger sequence number |

**Response Shape (detail):**

This endpoint is **DB-only** end to end. Both statements live in canonical SQL [`05_get_ledgers_by_sequence.sql`](../../../docs/architecture/database-schema/endpoint-queries/05_get_ledgers_by_sequence.sql):

- **Statement A** — `ledgers` row + `prev_sequence` / `next_sequence` via LATERAL on `idx_ledgers_closed_at`.
- **Statement B** — keyset-paginated read of the `transactions` partition for this ledger, with `WHERE ledger_sequence = $1 AND created_at = $closed_at`. The `closed_at = $closed_at` predicate is full equality (every transaction in a ledger shares the ledger's exact `closed_at`), so partition pruning is total — no range scan, no cross-partition work. Returns the seven DB-side fields of `TransactionListItem`.

The original 0167 framing of E5 as the "embedded list lives off-DB" exception relied on the per-ledger S3 JSON blob, a track abandoned by ADR 0029. With memo / other heavy fields removed from the `TransactionListItem` shape (they live only on the transaction detail endpoint's E3 heavy block), there is no archive XDR fetch on this endpoint at all. The pattern matches the production `transactions::list_transactions` exactly.

```json
{
  "sequence": 12345678,
  "hash": "abcdef...",
  "closed_at": "2026-03-20T12:00:00Z",
  "protocol_version": 21,
  "transaction_count": 150,
  "base_fee": 100,
  "prev_sequence": 12345677,
  "next_sequence": 12345679,
  "transactions": {
    "data": [
      {
        "hash": "7b2a8c...",
        "source_account": "GABC...XYZ",
        "successful": true,
        "fee_charged": 100,
        "created_at": "2026-03-20T12:00:00Z",
        "operation_count": 3
      }
    ],
    "page": {
      "cursor": "eyJ0cyI6IjIwMjYtMDMtMjBUMTI6MDA6MDBaIiwiaWQiOjEyM30",
      "limit": 20,
      "has_more": true
    }
  }
}
```

**Detail fields:**

| Field               | Type        | Source | Description                                                                                                       |
| ------------------- | ----------- | ------ | ----------------------------------------------------------------------------------------------------------------- |
| `sequence`          | number      | DB     | Ledger sequence number (primary key)                                                                              |
| `hash`              | string      | DB     | Ledger hash (64-char hex)                                                                                         |
| `closed_at`         | string      | DB     | ISO 8601 timestamp of ledger close                                                                                |
| `protocol_version`  | number      | DB     | Protocol version at close                                                                                         |
| `transaction_count` | number      | DB     | Number of transactions in this ledger                                                                             |
| `base_fee`          | number      | DB     | Base fee in stroops                                                                                               |
| `prev_sequence`     | number/null | DB     | Previous closed ledger by `closed_at` (null at chain tail)                                                        |
| `next_sequence`     | number/null | DB     | Next closed ledger by `closed_at` (null at chain head)                                                            |
| `transactions`      | object      | DB     | Paginated list of linked transactions; reuses `TransactionListItem` DTO from `crates/api/src/transactions/dto.rs` |

**Transactions DTO reuse:** the slim list-item shape is the public `TransactionListItem` already exported by the transactions module (task 0046). Do not redefine.

| Field             | Source | Notes                                                       |
| ----------------- | ------ | ----------------------------------------------------------- |
| `hash`            | DB     | `encode(t.hash, 'hex')`, lowercase                          |
| `ledger_sequence` | DB     | `t.ledger_sequence` (always equal to `:sequence` from path) |
| `source_account`  | DB     | StrKey from `accounts` join                                 |
| `successful`      | DB     | `t.successful`                                              |
| `fee_charged`     | DB     | `t.fee_charged`                                             |
| `created_at`      | DB     | `t.created_at`                                              |
| `operation_count` | DB     | `t.operation_count`                                         |

Memo and other heavy fields are NOT exposed on the list item — list endpoints stay DB-only by contract. Memo lives on the transaction detail endpoint (`GET /v1/transactions/{hash}`) inside the E3 heavy block.

**Embedded transactions[] pagination:**

- Real DB cursor pagination on `(created_at DESC, id DESC)` reusing the existing `TsIdCursor` codec from task 0043 — same convention as the top-level `GET /v1/transactions` list. The keyset predicate is inlined in canonical SQL 05 statement B (`($cursor_ts IS NULL OR (t.created_at, t.id) < ($cursor_ts, $cursor_id))`); no shared helper in `common/pagination.rs` — each resource owns its own predicate SQL.
- The query is statement B of canonical SQL 05: `WHERE ledger_sequence = $1 AND created_at = $closed_at AND (created_at, id) < ($cursor_ts, $cursor_id) ORDER BY created_at DESC, id DESC LIMIT $limit`.
- `closed_at` is read from statement A's result and threaded into statement B as a bind parameter — partition prune is full equality, single partition touched.
- The detail handler reuses the same `?limit=` / `?cursor=` query params (via `Pagination<TsIdCursor>` extractor) to drive embedded paging — detail itself is a single resource with no own pagination, so the standard params are free.
- Default `limit`: 20. Max: 100 (consistent with list endpoints).

### Behavioral Requirements

- Default ordering for the list endpoint is compound `(closed_at DESC, sequence DESC)`. The `idx_ledgers_closed_at` index alone would be enough for ordering, but pairing with `sequence` (PRIMARY KEY) makes the cursor totally ordered even in the unlikely tie case — same defensive pattern documented in canonical SQL 04.
- Linked transactions in detail use the standard `Paginated<T>` envelope (real DB cursor pagination on the `transactions` partition with `(created_at, id) DESC` keyset; see "Embedded transactions[] pagination" above).
- Closed ledgers are immutable: long-TTL caching is appropriate for everything except the chain head.
- Head-ledger detection: inspect `next_sequence` from statement A's LATERAL lookup. `next_sequence is None` ↔ this ledger is the chain head (no later row in `ledgers` yet). No extra `MAX(sequence)` query, no cross-module coupling to `network::cache`. The LATERAL is a side-cost of computing prev/next navigation, so the signal is free.
- Cache-Control headers are emitted **per-handler** via direct `HeaderValue` insertion (matching the `network/handlers.rs:24,63-68` pattern). No global tower middleware. API Gateway response caching configured in `infra/envs/*.json` is orthogonal and stays as-is.

### Caching

| Endpoint                 | Condition                            | TTL  | Notes                                                         |
| ------------------------ | ------------------------------------ | ---- | ------------------------------------------------------------- |
| `GET /ledgers`           | --                                   | 10s  | List changes as new ledgers close                             |
| `GET /ledgers/:sequence` | `next_sequence IS NOT NULL` (closed) | 300s | Immutable; safe to cache long                                 |
| `GET /ledgers/:sequence` | `next_sequence IS NULL` (chain head) | 10s  | Indexer may still be settling rows for the most-recent ledger |

Suggested header values:

- Closed: `Cache-Control: public, max-age=300`
- Head: `Cache-Control: public, max-age=10`
- List: `Cache-Control: public, max-age=10`

### Error Handling

Error envelope is **flat** per ADR 0008 (`crates/api/src/common/errors.rs::ErrorEnvelope`); `code` values are lowercase snake_case.

- 400 `invalid_id`: invalid sequence format (non-numeric, negative). Pagination param failures surface separately as `invalid_limit` / `invalid_cursor` from the shared `Pagination<TsIdCursor>` extractor.
- 404 `not_found`: ledger sequence not in DB.
- 500 `db_error`: unrecoverable database errors (logged server-side, generic message returned to client).

```json
{
  "code": "not_found",
  "message": "Ledger with sequence 99999999 not found.",
  "details": null
}
```

## Implementation Plan

### Step 1: Module scaffold

Create `crates/api/src/ledgers/` with the standard 4-file layout used by network/transactions/assets:

```
crates/api/src/ledgers/
├── mod.rs        # OpenApiRouter wiring (utoipa_axum::routes!)
├── dto.rs        # request + response types with ToSchema
├── queries.rs    # sqlx queries (uses common::cursor::TsIdCursor)
└── handlers.rs   # axum handlers + Cache-Control
```

Wire `ledgers::router()` into the main app router in `crates/api/src/main.rs`.

### Step 2: Cursor + pagination helper

Reuse `crate::common::cursor::TsIdCursor` for both lists. No new cursor type, no new pagination helper — `finalize_ts_id_page` covers the encode side and the keyset predicate is inlined in the canonical SQL files.

### Step 3: List endpoint — `GET /v1/ledgers`

Implement against canonical SQL [`04_get_ledgers_list.sql`](../../../docs/architecture/database-schema/endpoint-queries/04_get_ledgers_list.sql). Cursor pagination on `(closed_at DESC, sequence DESC)` via `TsIdCursor` (ts=closed_at, id=sequence). Response envelope `Paginated<LedgerListItem>`. Cache-Control `public, max-age=10`.

### Step 4: Detail endpoint — `GET /v1/ledgers/:sequence`

Implement against canonical SQL [`05_get_ledgers_by_sequence.sql`](../../../docs/architecture/database-schema/endpoint-queries/05_get_ledgers_by_sequence.sql) — two DB statements, no archive XDR fetch.

The handler accepts a `Pagination<TsIdCursor>` extractor reading `?limit=` / `?cursor=` from the request — these drive the embedded transactions[] pagination. Detail itself is a single resource with no own pagination, so the standard params are free for the embedded list (no naming collision).

1. **Statement A — header.** Resolve `:sequence` against `ledgers` + LATERAL prev/next on `idx_ledgers_closed_at`. If no row, return 404 `not_found` and skip the rest. Carry the row's `closed_at` forward.
2. **Statement B — embedded transactions[].** Keyset-paginated read of the `transactions` partition with `WHERE ledger_sequence = $1 AND created_at = $closed_at`, threading the validated `cursor` / `limit + 1` from the extractor. Ordering `(created_at DESC, id DESC)` via the same `TsIdCursor` codec used by the top-level transactions list. Project the seven DB-side fields of `TransactionListItem`.

Compose the final response: header fields + `prev_sequence` + `next_sequence` + nested `transactions: Paginated<TransactionListItem>`.

### Step 5: Cache-Control + head-ledger detection

In handler:

1. Inspect `next_sequence` from statement A's row. `next_sequence is None` ↔ this ledger is the chain head (no later ledger has closed yet, so the indexer may still be settling).
2. If head: emit `Cache-Control: public, max-age=10`.
3. Else (closed, immutable): emit `Cache-Control: public, max-age=300`.

This avoids any cross-module coupling to `network::cache` and any extra `MAX(sequence)` query — the LATERAL lookup on `idx_ledgers_closed_at` already produces the signal as a side effect of computing prev/next navigation.

Pattern: const `HeaderValue` per case (mirror of `network/handlers.rs:24,63-68`).

### Step 6: OpenAPI + error envelope

All DTOs derive `utoipa::ToSchema`. Error responses use the existing `ErrorEnvelope` from `crates/api/src/common/errors.rs` (flat, lowercase `code`, per ADR 0008).

### Step 7: Tests

- Unit: `TsIdCursor` round-trip + page truncation are already covered by `common::cursor::tests` and `common::pagination::tests` — list endpoints reuse those helpers verbatim, so no per-endpoint duplication.
- Integration (in `crates/api/src/tests_integration.rs`):
  - validation short-circuits before DB: `ledgers_invalid_sequence_returns_400_envelope`, `ledgers_list_invalid_limit_returns_envelope_before_db`, `ledgers_list_invalid_cursor_returns_envelope_before_db`
  - real-DB envelope shape + Cache-Control: `ledgers_list_returns_paginated_envelope_against_real_db`
  - real-DB cursor traversal no overlap: `ledgers_cursor_round_trip_no_overlap_against_real_db`
  - real-DB 404 path: `ledgers_detail_unknown_sequence_returns_404_against_real_db`
  - real-DB head-vs-closed Cache-Control + full detail field shape + `next_sequence is None` head signal: `ledgers_detail_returns_header_and_cache_control_against_real_db`
- OpenAPI smoke (in `main::tests`): `api_docs_json_contains_ledgers_paths`.

### Step 8: Docs

Per ADR 0032: every shape-of-system change requires a doc update.

- `docs/architecture/backend/backend-overview.md` — confirm §6.2 / §6.3 ledger entries match shipped contract; update if drift.
- `docs/architecture/database-schema/endpoint-queries/05_get_ledgers_by_sequence.sql` already updated 2026-04-28: rewritten as a two-statement file (header + transactions partition query, no memo projection) with the SUPERSESSION NOTE explaining how E5 lines up with the production `GET /v1/transactions` pattern (DB structural fields + single archive XDR memo fetch per ADR 0029) and why the original 0167 S3-blob plan no longer applies.
- N/A for `04_get_ledgers_list.sql` — list contract unchanged.

## Acceptance Criteria

- [x] `GET /v1/ledgers` returns `Paginated<LedgerListItem>` ordered by `(closed_at DESC, sequence DESC)`.
- [x] List response uses ADR 0008 envelope: `{ data, page: { cursor, limit, has_more } }`.
- [x] Cursor pagination on compound key `(closed_at, sequence)` via `TsIdCursor` (ts=closed_at, id=sequence); predicate inlined in canonical SQL 04 as `(closed_at, sequence) < ($cursor_ts, $cursor_id)`.
- [x] `GET /v1/ledgers/:sequence` returns header fields + `prev_sequence` + `next_sequence` + `transactions: Paginated<TransactionListItem>`.
- [x] Detail endpoint accepts `?limit=` / `?cursor=` query params to drive embedded transactions pagination — same query-param shape as the list endpoints.
- [x] Embedded `transactions[]` are sourced exclusively from the DB `transactions` partition with full equality partition prune (`created_at = $closed_at`). No archive XDR fetch on this endpoint.
- [x] Embedded `transactions[]` use the `TsIdCursor` codec from task 0043 (same convention as `GET /v1/transactions`).
- [x] `TransactionListItem` is reused from `crates/api/src/transactions/dto.rs` — no duplicate DTO. List item carries DB-only fields (no `memo_type` / `memo` — those live on the transaction detail endpoint).
- [x] The original 0167 plan to source embedded txs from a per-ledger S3 JSON blob is NOT used (abandoned by ADR 0029).
- [x] `Cache-Control: public, max-age=300` for non-head ledgers; `public, max-age=10` for the head ledger and for the list endpoint.
- [x] Head detection uses `next_sequence is None` from statement A's LATERAL lookup (no extra query, no cross-module cache).
- [x] 404 `not_found` (flat ADR 0008 envelope, lowercase `code`) for non-existent ledger sequences.
- [x] 400 `invalid_id` for invalid sequence format (non-numeric, negative); pagination param failures surface as `invalid_limit` / `invalid_cursor` from the shared extractor.
- [x] OpenAPI schema generated correctly (utoipa) for all responses.
- [x] Module layout matches sibling modules (`mod / dto / queries / handlers`).
- [x] Tests cover cursor traversal (both lists), 404, head vs closed Cache-Control, and prev/next at chain head/tail (NULL handling).
- [x] Docs updated per ADR 0032 (see Step 8).

## Implementation Notes

**Files shipped:**

- `crates/api/src/ledgers/mod.rs` — 28 lines, OpenApiRouter wiring for `list_ledgers` + `get_ledger`
- `crates/api/src/ledgers/dto.rs` — 61 lines, `LedgerListItem` (also `sqlx::FromRow` target) + `LedgerDetailResponse`
- `crates/api/src/ledgers/queries.rs` — 210 lines, three static `query_as` calls + `From<LedgerTxRow> for TransactionListItem`
- `crates/api/src/ledgers/handlers.rs` — 201 lines, two handlers + Cache-Control consts (SHORT 10s / LONG 300s)

**Module wiring:** `mod ledgers;` + `.nest("/v1", ledgers::router())` added to `crates/api/src/main.rs`. `ledgers::router()` also registered in `tests_integration::build_app` so DB-gated tests can hit it.

**Sibling module impact:** `transactions::list_transactions` simplified — archive XDR fetch loop + `extract_e3_memo` calls removed. `TransactionListItem` DTO trimmed: dropped `memo_type` / `memo` fields. `extract_e3_memo` function deleted (no callers).

**Doc updates per ADR 0032:**

- `docs/architecture/database-schema/endpoint-queries/05_get_ledgers_by_sequence.sql` rewritten as DB-only two-statement file with SUPERSESSION NOTE explaining ADR 0029 fallout. LATERAL prev/next keyed by `sequence` (PK index-only scan).
- `04_get_ledgers_list.sql` — N/A, list contract unchanged.
- `backend-overview.md` §6.5 already lists ledger endpoints, no drift.

**Tests (90 pass / 5 ignored network):**

- `main::tests::api_docs_json_contains_ledgers_paths` — OpenAPI smoke
- 7 integration tests in `tests_integration.rs`:
  - `ledgers_invalid_sequence_returns_400_envelope` (no DB)
  - `ledgers_list_invalid_limit_returns_envelope_before_db` (no DB)
  - `ledgers_list_invalid_cursor_returns_envelope_before_db` (no DB)
  - `ledgers_list_returns_paginated_envelope_against_real_db`
  - `ledgers_cursor_round_trip_no_overlap_against_real_db`
  - `ledgers_detail_unknown_sequence_returns_404_against_real_db`
  - `ledgers_detail_returns_header_and_cache_control_against_real_db`

**Senior-review pass refactors:**

- `LedgerRow` collapsed into public `LedgerListItem` (identical fields → single struct with `sqlx::FromRow + Serialize + Deserialize + ToSchema`); saves a struct + identity mapper.
- Manual tuple destructuring in `query_as` → `#[derive(sqlx::FromRow)]` on row structs (~63 lines saved in queries.rs alone).
- `From<LedgerTxRow> for TransactionListItem` impl replaces inline closure mapper.
- Cache-Control consts renamed `RECENT/CLOSED` → `SHORT/LONG` (the short value is also used by the list, so positional naming is more accurate; comments now explain the 10s ceiling is pinned by the API Gateway TTL config, NOT the ~5s ledger close cadence).
- LATERAL prev/next swapped from `idx_ledgers_closed_at` (secondary) to `ledgers` PK on `sequence` for index-only scan.

## Issues Encountered

- **0167 spec drift on E5 framing** — task 0167 (delivered 2026-04-27) framed `GET /ledgers/:sequence` as "embedded list lives off-DB" with a per-ledger S3 JSON blob. ADR 0029 had landed 6 days earlier (2026-04-21) abandoning the S3 blob track but 0167 was authored without integrating that decision. Spec 0047 + canonical SQL 05 were realigned in this task — DB-only with SUPERSESSION NOTE on SQL 05 explaining the abandoned plan.
- **`memo_type` / `memo` are NOT in DB** — initially assumed they were table columns and could be SELECTed. Sibling `transactions::list_transactions` DOES enrich them via `extract_e3_memo` (archive XDR fetch). After clarification with the human, decision was to drop them from `TransactionListItem` entirely — list endpoints stay DB-only by contract; memo lives on the transaction detail endpoint's E3 heavy block.
- **Embedded txs cursor unreachable in first cut** — first implementation hardcoded `cursor=None` for embedded transactions inside `get_ledger`. The response shipped a cursor in the page envelope but the client had no way to send it back. Fixed by adding a `Pagination<TsIdCursor>` extractor to the detail handler — detail itself is a single resource so `?limit=` / `?cursor=` are free to drive embedded pagination.

## Design Decisions

### From Plan

1. **Two endpoints, DB-only.** Per spec — list paginated, detail with embedded paginated transactions.
2. **Reuse `TransactionListItem` DTO.** No duplicate slim DTO; list rows shared cross-module.
3. **Per-handler Cache-Control.** Mirror of `network/handlers.rs:24,63-68` pattern. No middleware layer.
4. **`Pagination<TsIdCursor>` extractor on both endpoints.** Project-default cursor codec; list maps `(closed_at, sequence)` → `(ts, id)` because cursor format is opaque per ADR 0008.

### Emerged

5. **Head-ledger detection via `next_sequence is None`.** Original spec assumed `NetworkStats.latest_ledger_sequence` (cross-module coupling to network cache). Switched mid-implementation to inspecting the LATERAL `next_sequence` column already produced by the header query. No extra query, no cache coupling — the signal is a free side-cost of computing prev/next navigation. This was an autonomous decision; cleaner than the spec's original.
6. **Drop `memo_type` / `memo` from `TransactionListItem`.** Required clarification from the human after second-pass review surfaced "memo doesn't exist as DB columns, was being archive-fetched per row in transactions list". Final shape: list endpoints stay DB-only, memo only on tx detail. This emerged after my own first-cut had wired an archive fetch into `get_ledger` for memo enrichment — the slim shape removed that fetch entirely.
7. **LATERAL prev/next on PK instead of `idx_ledgers_closed_at`.** Senior-review pass switched the LATERAL keying from `closed_at` (secondary index, requires heap fetch for `SELECT sequence`) to `sequence` (PK, index-only scan). Marginal perf gain but semantically more obvious; SQL 05 canonical was updated to match.
8. **Refactor pass: drop `LedgerRow` struct.** Initial implementation followed the sibling-module split "internal DB row + public DTO". For ledgers the two structs had identical fields, so the split was pure ceremony. Collapsed into a single `LedgerListItem` with both `sqlx::FromRow` and the public derives. Saves a struct + an identity mapper.
9. **`#[derive(sqlx::FromRow)]` over manual tuple destructuring.** First implementation used `query_as::<_, (i64, String, ...)>` + manual `.map()`. Senior-review pass switched to FromRow derives — idiomatic, ~63 lines saved, less drift potential when adding columns.
10. **Cache-Control const naming `SHORT`/`LONG`.** Initial `RECENT`/`CLOSED` was misleading (the short value is also used by the list endpoint, which is not "recent ledger"). Renamed for clarity; comment explains 10s value is pinned by API Gateway TTL config, not by the ~5s Stellar ledger cadence.

## Future Work

- Composite index `transactions (ledger_sequence, id DESC)` would eliminate the in-memory sort step on the embedded transactions query. Marginal for typical-size ledgers (50–200 tx). Belongs to existing **task 0132** (missing-index findings) — flagged there, no separate spawn.
- Reindex after upstream indexer bug fixes (**0168** / **0169** / **0170**) will refresh historical `transaction_count` and downstream tx fields. Long Cache-Control (300s) on closed ledger detail means stale rows persist for up to 5 minutes after a reindex; cache purge is the operational mitigation. Out of scope here; documented inline on the `CACHE_CONTROL_LONG` const.

## Notes

- Ledgers is one of the simplest modules. List = single DB query, no filters. Detail = "the transactions list scoped to one ledger" + the ledger header. Both endpoints are pure DB-only.
- Canonical SQL files (04, 05) under `docs/architecture/database-schema/endpoint-queries/` are the source of truth for read shape — port them to `sqlx` rather than rewriting from this spec's response tables.
