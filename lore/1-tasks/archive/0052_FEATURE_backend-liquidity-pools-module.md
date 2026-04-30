---
id: '0052'
title: 'Backend: Liquidity Pools module (list + detail + transactions + chart)'
type: FEATURE
status: completed
related_adr: ['0005', '0024', '0027', '0031']
related_tasks: ['0023', '0043', '0092']
tags: [layer-backend, liquidity-pools, charts]
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
  - date: 2026-04-29
    status: active
    who: karolkow
    note: 'Activated — bundled with 0051 (NFTs) on shared branch, parallel module shape to 0048 accounts.'
  - date: 2026-04-29
    status: active
    who: karolkow
    note: 'Spec refresh vs current schema (ADR 0024/0027/0031): pool_id is BYTEA(32) hex-encoded externally; assets stored as flat columns (asset_X_type/code/issuer_id) and assembled into JSONB shape in handler; reserves/total_shares/tvl/last_updated_ledger live in liquidity_pool_snapshots, not on liquidity_pools — detail joins latest snapshot. Module already exists with participants endpoint (task 0126); list/detail/transactions/chart still TODO.'
  - date: 2026-04-30
    status: completed
    who: karolkow
    note: >
      Implemented 4 endpoints (list/detail/transactions/chart) bundled with
      task 0051 on shared branch. Extended crates/api/src/liquidity_pools/
      module (~430 LOC handlers + ~390 LOC queries). Wire shapes pinned to
      canonical SQL 18/19/20/21. New common helpers: path::pool_id_hex,
      filters::parse_iso8601[_opt], filters::reject_sql_wildcards[_opt].
      Dropped snapshot freshness windows after auditing ingestion (snapshots
      are state-change-driven so latest IS current). Chart params all
      optional with interval-tuned defaults (1h→7d / 1d→90d / 1w→104w).
      MAX_CHART_BUCKETS=1000 DoS guard. Mixed asset filter pairing
      validator added (canonical 18 said "API validates upstream").
      143 tests passing (+12 LP integration tests).
---

# Backend: Liquidity Pools module (list + detail + transactions + chart)

## Summary

Implement the Liquidity Pools module providing pool listing with asset/TVL filters, pool detail, pool transaction history (deposits, withdrawals, trades), and time-series chart data. The module must separate pool current-state queries (liquidity_pools table) from chart queries (liquidity_pool_snapshots table).

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Backlog

**Current state:** Not started. Depends on tasks 0023 (bootstrap), 0043 (pagination).

## Context

Liquidity pools combine current-state reads with historical aggregate reads. The pool detail serves current reserves and TVL, while the chart endpoint serves time-series data from the snapshots table. Pool transaction history is derived from transactions, operations, and Soroban events rather than a dedicated pool-transactions table.

### API Specification

**Location:** `crates/api/src/liquidity_pools/` (already scaffolded; participants endpoint from task 0126 is mounted, list/detail/transactions/chart added by this task).

---

#### GET /v1/liquidity-pools

**Method:** GET

**Path:** `/liquidity-pools`

**Query Parameters:**

| Parameter         | Type   | Default | Description              |
| ----------------- | ------ | ------- | ------------------------ |
| `limit`           | number | 20      | Items per page (max 100) |
| `cursor`          | string | null    | Opaque pagination cursor |
| `filter[assets]`  | string | null    | Filter by asset pair     |
| `filter[min_tvl]` | number | null    | Minimum TVL threshold    |

**Response Shape (list):**

```json
{
  "data": [
    {
      "pool_id": "abcdef1234...",
      "asset_a": { "type": "native" },
      "asset_b": {
        "type": "credit_alphanum4",
        "code": "USDC",
        "issuer": "GCNY...ABC"
      },
      "fee_bps": 30,
      "reserves": { "a": "1000000.0000000", "b": "500000.0000000" },
      "total_shares": "750000.0000000",
      "tvl": "1500000.0000000"
    }
  ],
  "pagination": {
    "next_cursor": "eyJpZCI6Mn0=",
    "has_more": true
  }
}
```

---

#### GET /v1/liquidity-pools/:id

**Method:** GET

**Path:** `/liquidity-pools/:id`

**Path Parameters:**

| Parameter | Type   | Description           |
| --------- | ------ | --------------------- |
| `id`      | string | Pool ID (64-char hex) |

**Response Shape:**

```json
{
  "pool_id": "abcdef1234...",
  "asset_a": { "type": "native" },
  "asset_b": {
    "type": "credit_alphanum4",
    "code": "USDC",
    "issuer": "GCNY...ABC"
  },
  "fee_bps": 30,
  "reserves": { "a": "1000000.0000000", "b": "500000.0000000" },
  "total_shares": "750000.0000000",
  "tvl": "1500000.0000000",
  "created_at_ledger": 10000000,
  "last_updated_ledger": 12345678
}
```

**Detail fields:**

| Field                 | Type           | Description                               |
| --------------------- | -------------- | ----------------------------------------- |
| `pool_id`             | string         | Pool ID (64-char primary key)             |
| `asset_a`             | object (JSONB) | First asset in the pair                   |
| `asset_b`             | object (JSONB) | Second asset in the pair                  |
| `fee_bps`             | number         | Fee in basis points (NOT NULL in schema)  |
| `reserves`            | object (JSONB) | Current reserves                          |
| `total_shares`        | string         | Total pool shares                         |
| `tvl`                 | string         | Total value locked                        |
| `created_at_ledger`   | number         | Ledger where pool was created             |
| `last_updated_ledger` | number         | Most recent ledger with pool state change |

**Storage note (ADR 0024/0027/0031):** `liquidity_pools` only carries static metadata (`pool_id BYTEA(32)`, `asset_X_type/code/issuer_id`, `fee_bps`, `created_at_ledger`). Dynamic fields — `reserves`, `total_shares`, `tvl`, `last_updated_ledger` — come from the **latest** `liquidity_pool_snapshots` row for that pool (`ORDER BY ledger_sequence DESC LIMIT 1`). `pool_id` is hex-encoded (64-char lowercase) for the API; `asset_a`/`asset_b` JSONB shapes are assembled in the handler from the flat asset columns + `accounts`/`assets` joins.

---

#### GET /v1/liquidity-pools/:id/transactions

**Method:** GET

**Path:** `/liquidity-pools/:id/transactions`

**Path Parameters:**

| Parameter | Type   | Description |
| --------- | ------ | ----------- |
| `id`      | string | Pool ID     |

**Query Parameters:**

| Parameter | Type   | Default | Description              |
| --------- | ------ | ------- | ------------------------ |
| `limit`   | number | 20      | Items per page (max 100) |
| `cursor`  | string | null    | Opaque pagination cursor |

**Response Shape:**

```json
{
  "data": [
    {
      "hash": "7b2a8c...",
      "type": "deposit",
      "source_account": "GABC...XYZ",
      "successful": true,
      "ledger_sequence": 12345678,
      "created_at": "2026-03-20T12:00:00Z"
    }
  ],
  "pagination": {
    "next_cursor": "eyJpZCI6MTIzfQ==",
    "has_more": true
  }
}
```

**Transaction types:** deposits, withdrawals, trades. Derived from transactions + operations + soroban_events, NOT a separate table.

---

#### GET /v1/liquidity-pools/:id/chart

**Method:** GET

**Path:** `/liquidity-pools/:id/chart`

**Path Parameters:**

| Parameter | Type   | Description |
| --------- | ------ | ----------- |
| `id`      | string | Pool ID     |

**Query Parameters:**

| Parameter  | Type   | Required | Description                     |
| ---------- | ------ | -------- | ------------------------------- |
| `interval` | string | yes      | Time interval: `1h`, `1d`, `1w` |
| `from`     | string | yes      | Start time (ISO 8601 timestamp) |
| `to`       | string | yes      | End time (ISO 8601 timestamp)   |

**Response Shape:**

```json
{
  "pool_id": "abcdef1234...",
  "interval": "1d",
  "from": "2026-03-01T00:00:00Z",
  "to": "2026-03-20T00:00:00Z",
  "data_points": [
    {
      "timestamp": "2026-03-01T00:00:00Z",
      "tvl": "1500000.0000000",
      "volume": "250000.0000000",
      "fee_revenue": "750.0000000",
      "reserves": { "a": "1000000.0000000", "b": "500000.0000000" },
      "total_shares": "750000.0000000"
    }
  ]
}
```

**Data point fields:**

| Field          | Type           | Description                            |
| -------------- | -------------- | -------------------------------------- |
| `timestamp`    | string         | ISO 8601 timestamp for this data point |
| `tvl`          | string         | Total value locked at this point       |
| `volume`       | string         | Trading volume in the interval         |
| `fee_revenue`  | string         | Fee revenue in the interval            |
| `reserves`     | object (JSONB) | Reserves at this point                 |
| `total_shares` | string         | Total shares at this point             |

**Data source:** `liquidity_pool_snapshots` table. NOT computed from raw transactions at query time.

**Validation:**

- `interval` must be one of: `1h`, `1d`, `1w`
- `from` and `to` must be valid ISO 8601 timestamps
- `from` must be before `to`

### Behavioral Requirements

- Pool detail current-state values (`reserves`, `total_shares`, `tvl`, `last_updated_ledger`) come from the latest `liquidity_pool_snapshots` row; static fields (`pool_id`, assets, `fee_bps`, `created_at_ledger`) come from `liquidity_pools`
- Pool transactions derived from transactions + operations + soroban_events
- Chart data from pre-computed liquidity_pool_snapshots (interval aggregation)
- Asset pair response payloads are JSONB **shapes** assembled in the handler from flat schema columns (`asset_X_type/code/issuer_id`); may span classic and Soroban-native
- `pool_id` rendered as 64-char lowercase hex externally, stored as `BYTEA(32)` (ADR 0024)
- Validate interval parameter strictly

### Caching

| Endpoint                                | TTL     | Notes                                 |
| --------------------------------------- | ------- | ------------------------------------- |
| `GET /liquidity-pools`                  | 5-15s   | List changes as pools update          |
| `GET /liquidity-pools/:id`              | 5-15s   | Pool state updates with new ledgers   |
| `GET /liquidity-pools/:id/transactions` | 5-15s   | New transactions appear               |
| `GET /liquidity-pools/:id/chart`        | 60-120s | Snapshot data changes less frequently |

### Error Handling

- 400: Invalid pool ID format, invalid interval, invalid from/to, from > to
- 404: Pool not found
- 500: Database errors

```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "interval must be one of: 1h, 1d, 1w"
  }
}
```

## Implementation Plan

### Step 1: Route + handler setup

Module already exists at `crates/api/src/liquidity_pools/` (participants from task 0126). Add new `dto`/`handlers`/`queries` items and register the new routes alongside the existing `list_participants`.

### Step 2: List Endpoint

Implement `GET /liquidity-pools` with cursor pagination and filter[assets]/filter[min_tvl] support. List rows JOIN latest snapshot for `reserves`/`total_shares`/`tvl`; hex-encode `pool_id`; assemble asset JSONB from flat columns.

### Step 3: Detail Endpoint

Implement `GET /liquidity-pools/:id` joining `liquidity_pools` (static) with the latest `liquidity_pool_snapshots` row (dynamic). Decode hex `pool_id` → BYTEA at the boundary (reuse `is_valid_pool_id_hex` from existing handlers).

### Step 4: Transactions Endpoint

Implement `GET /liquidity-pools/:id/transactions` deriving pool transactions from transactions, operations, and soroban_events.

### Step 5: Chart Endpoint

Implement `GET /liquidity-pools/:id/chart` querying `liquidity_pool_snapshots` with interval aggregation, from/to filtering, and strict interval validation.

## Acceptance Criteria

- [x] `GET /v1/liquidity-pools` returns paginated pool list
- [x] `GET /v1/liquidity-pools/:id` returns pool detail (static fields from `liquidity_pools` + dynamic fields from latest `liquidity_pool_snapshots` row)
- [x] `GET /v1/liquidity-pools/:id/transactions` returns paginated pool transactions
- [x] `GET /v1/liquidity-pools/:id/chart` returns time-series data points
- [x] Chart data sourced from liquidity_pool_snapshots, not computed at query time
- [x] Interval validated: only 1h, 1d, 1w accepted
- [x] from/to validated as ISO timestamps, from must be before to
- [x] filter[assets] and filter[min_tvl] work correctly — `filter[assets]` shipped as four per-leg keys (`asset_a_code` / `asset_a_issuer` / `asset_b_code` / `asset_b_issuer`) mirroring canonical SQL `18_*.sql` four-input shape; pairing rule `(code, issuer)` enforced. `filter[min_tvl]` accepted as decimal string against latest-snapshot TVL (currently NULL until TVL ingestion lands)
- [x] Pool transactions derived from transactions + operations + events
- [x] Pool current state separate from chart queries
- [x] `pool_id` is accepted from clients as a 64-character lowercase hex string in request paths/cursors, validated and decoded to `BYTEA(32)` for internal SQL queries, and re-encoded to a 64-character lowercase hex string in responses; malformed values (wrong length, uppercase, non-hex chars) are rejected at the API boundary with `400 ErrorEnvelope { code: "invalid_pool_id" }`. The DB column is `BYTEA(32)`; only the wire surface is hex.
- [x] Asset JSONB shapes assembled in handler from flat schema columns + `accounts`/`assets` joins
- [x] Standard pagination and error envelopes
- [x] 404 for non-existent pools

## Implementation Notes

- Extended existing module `crates/api/src/liquidity_pools/` (was scaffolded by task 0126 for participants endpoint). Added `list_pools`, `get_pool`, `list_pool_transactions`, `get_pool_chart` (~430 LOC handler + ~390 LOC queries).
- Wire shapes pinned to canonical SQL `18/19/20/21_*.sql`.
- Cursors: `PoolListCursor { created_at_ledger, pool_id_hex }` (list) + `TsIdCursor` (transactions).
- New `path::pool_id_hex` helper extracted to `common::path` (used 4× in LP handlers + cursor sanity).
- New const `INVALID_POOL_ID` in `common::errors`.
- Chart endpoint: all params optional with sensible defaults (`interval=1d`, `to=now()`, `from=to - {7d|90d|104w}` per interval). Bucket cap `MAX_CHART_BUCKETS=1000` to bound `GROUP BY + ARRAY_AGG`.
- Mixed asset filter pairing rejected with 400 (canonical 18 §46-49 says "API validates upstream").

## Issues Encountered

- **Snapshot freshness windows turned out to be non-essential**: canonical SQL 18/19 had `:snapshot_window` parameter as `e.g. '1 day' / '7 days'` examples. After auditing snapshot ingestion (`xdr_parser::extract_liquidity_pools`), confirmed snapshots are state-change-driven (only on `created`/`updated`/`restored` ledger entry changes) — so the LATEST snapshot row IS the actual current on-chain reserves regardless of age. Plus `tvl`/`volume`/`fee_revenue` are always NULL today (TVL ingestion not implemented). Window filter would NULL out otherwise-accurate reserves. Dropped windows entirely; clients read `latest_snapshot_at` if they care about freshness. Updated canonical SQL 18 + 19 + README to reflect.

- **`filter[min_tvl]` is dead today**: spec requires it, implementation has it, but TVL is NULL on every snapshot until TVL-ingestion task lands. Filter currently excludes all pools when active. Documented in canonical 18 + AC text. Forward-compat surface ready.

- **Chart bucket cap (`MAX_CHART_BUCKETS = 1000`) is a senior judgment call**: not in original spec, added as DoS guard. Without it, `?interval=1h&from=2016-01-01&to=2026-01-01` → ~87,600 buckets → expensive `GROUP BY + ARRAY_AGG`.

- **Canonical SQL 21 chart `WITH bucket_keyword` had a misleading comment**: claimed CASE-without-ELSE _"fail loudly"_ on bad interval, but `date_trunc(NULL, ts) → NULL` (silent garbage, not error). Corrected canonical SQL comment + added `debug_assert!` in Rust to catch allowlist drift in tests.

- **Mixed asset filter input**: canonical 18 §46-49 said "API validates upstream" but the validator was missing. Added handler-side check enforcing `(code, issuer)` paired or both omitted (classic identity).

## Design Decisions

### From Plan

1. **Pool detail = static fields from `liquidity_pools` + dynamic fields from latest `liquidity_pool_snapshots` row** (per ADR 0024/0027/0031). Static = pool_id/asset legs/fee/created_at_ledger; Dynamic = reserves/total_shares/tvl/volume/fee_revenue/latest_snapshot_at.

2. **`pool_id` = `BYTEA(32)` in DB, 64-char lowercase hex on wire** (ADR 0024). Validation + `decode($N::varchar, 'hex')` at the API/SQL boundary; `encode(... , 'hex')` on response.

3. **Chart from `liquidity_pool_snapshots`** with `date_trunc + ARRAY_AGG[1] + SUM` aggregation, NOT computed from raw transactions.

### Emerged

4. **Per-leg filter keys instead of `filter[assets]` composite**: spec used `filter[assets]` shorthand. Canonical SQL `18_*.sql` accepts 4 separate inputs (`asset_a_code`/`asset_a_issuer`/`asset_b_code`/`asset_b_issuer`). Implementing canonical = 4 keys; composite would need server-side parser. Backend-overview updated to reflect.

5. **All chart params optional with sensible defaults**: original spec required `interval`/`from`/`to`. Made all optional — bare `?` request returns useful chart (last 90 days at 1d granularity). Defaults are interval-tuned: `1h→7d`, `1d→90d`, `1w→104w` (all under 1000-bucket cap).

6. **`MAX_CHART_BUCKETS = 1000`**: DoS guard. Subjective threshold — covers all realistic UI ranges (41 days @1h, 2.7 years @1d, 19 years @1w).

7. **Dropped snapshot freshness windows entirely**: canonical 18/19 had `:snapshot_window` parameter (`e.g. '1 day' / '7 days'` examples). After verifying snapshot ingestion is state-change-driven (latest = current accurate state), windows added complexity without benefit. Clients use `latest_snapshot_at` to judge freshness. Participants endpoint (task 0126) keeps its hardcoded 7-day window — out of scope for this task.

8. **Mixed asset filter pairing validator**: canonical 18 §46-49 said "API validates upstream" but no implementation existed. Added — without it `?filter[asset_a_code]=USDC` (alone) would match all USDC-coded pools regardless of issuer (incl. fake/scam USDC).

9. **`NUMERIC ::text` casting in SQL projections** (`reserve_a::text` etc.): preserves `NUMERIC(28,7)` precision over the wire. Without it, sqlx → serde_json round-trip via f64 loses precision on the 7th decimal. Pattern from `participants` endpoint (task 0126).

10. **`COALESCE(ops.operation_types, ARRAY[]::text[])` in transactions LATERAL**: defensive against NULL when LATERAL returns no row. Canonical 20 didn't have it; pattern from `assets/queries.rs:258`.

11. **Promoted `filters::parse_iso8601` + `parse_iso8601_opt` to `common::filters`**: chart endpoint only consumer today, but pattern symmetric to `strkey_opt` / `parse_enum_opt`. Future timestamp filters (e.g. transactions list `?from=&to=`) reuse.

12. **Extracted `path::pool_id_hex` to `common::path`**: 4 path validators in LP handlers used identical inline shape check. Promoted matching `parse_hash` / `strkey` / `sequence` precedent. Doc table at top of `path.rs` updated.

## Notes

- The chart endpoint is the most complex part of this module, requiring snapshot table queries with interval aggregation.
- Pool transaction derivation from events is similar to the NFT transfer derivation pattern.
- Asset pair JSONB may contain either classic or Soroban-native asset identities.
