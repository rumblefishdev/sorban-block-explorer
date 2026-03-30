---
id: '0052'
title: 'Backend: Liquidity Pools module (list + detail + transactions + chart)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0043']
tags: [layer-backend, liquidity-pools, charts]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: Liquidity Pools module (list + detail + transactions + chart)

## Summary

Implement the Liquidity Pools module providing pool listing with asset/TVL filters, pool detail, pool transaction history (deposits, withdrawals, trades), and time-series chart data. The module must separate pool current-state queries (liquidity_pools table) from chart queries (liquidity_pool_snapshots table).

## Status: Backlog

**Current state:** Not started. Depends on tasks 0023 (bootstrap), 0043 (pagination).

## Context

Liquidity pools combine current-state reads with historical aggregate reads. The pool detail serves current reserves and TVL, while the chart endpoint serves time-series data from the snapshots table. Pool transaction history is derived from transactions, operations, and Soroban events rather than a dedicated pool-transactions table.

### API Specification

**Location:** `apps/api/src/liquidity-pools/`

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
| `fee_bps`             | number or null | Fee in basis points                       |
| `reserves`            | object (JSONB) | Current reserves                          |
| `total_shares`        | string         | Total pool shares                         |
| `tvl`                 | string         | Total value locked                        |
| `created_at_ledger`   | number         | Ledger where pool was created             |
| `last_updated_ledger` | number         | Most recent ledger with pool state change |

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

- Separate pool current-state queries (liquidity_pools table) from chart queries (snapshots table)
- Pool transactions derived from transactions + operations + soroban_events
- Chart data from pre-computed liquidity_pool_snapshots
- Asset pair payloads are JSONB (may span classic and Soroban-native)
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

### Step 1: Module Scaffolding

Create `apps/api/src/liquidity-pools/` with module, controller, service, and DTOs.

### Step 2: List Endpoint

Implement `GET /liquidity-pools` with cursor pagination and filter[assets]/filter[min_tvl] support.

### Step 3: Detail Endpoint

Implement `GET /liquidity-pools/:id` querying the `liquidity_pools` table.

### Step 4: Transactions Endpoint

Implement `GET /liquidity-pools/:id/transactions` deriving pool transactions from transactions, operations, and soroban_events.

### Step 5: Chart Endpoint

Implement `GET /liquidity-pools/:id/chart` querying `liquidity_pool_snapshots` with interval aggregation, from/to filtering, and strict interval validation.

## Acceptance Criteria

- [ ] `GET /v1/liquidity-pools` returns paginated pool list
- [ ] `GET /v1/liquidity-pools/:id` returns pool detail
- [ ] `GET /v1/liquidity-pools/:id/transactions` returns paginated pool transactions
- [ ] `GET /v1/liquidity-pools/:id/chart` returns time-series data points
- [ ] Chart data sourced from liquidity_pool_snapshots, not computed at query time
- [ ] Interval validated: only 1h, 1d, 1w accepted
- [ ] from/to validated as ISO timestamps, from must be before to
- [ ] filter[assets] and filter[min_tvl] work correctly
- [ ] Pool transactions derived from transactions + operations + events
- [ ] Pool current state separate from chart queries
- [ ] Standard pagination and error envelopes
- [ ] 404 for non-existent pools

## Notes

- The chart endpoint is the most complex part of this module, requiring snapshot table queries with interval aggregation.
- Pool transaction derivation from events is similar to the NFT transfer derivation pattern.
- Asset pair JSONB may contain either classic or Soroban-native asset identities.
