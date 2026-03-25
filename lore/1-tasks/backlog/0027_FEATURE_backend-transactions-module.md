---
id: '0027'
title: 'Backend: Transactions module (list + detail + filters)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0024', '0025']
tags: [layer-backend, transactions, filters, xdr]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: Transactions module (list + detail + filters)

## Summary

Implement the Transactions module providing paginated transaction listing with filters and dual-mode transaction detail (normal and advanced views). This is the central activity-browsing module of the explorer API, handling the most complex response shaping including operation trees, events, and raw XDR for advanced inspection.

## Status: Backlog

**Current state:** Not started. Depends on tasks 0023 (bootstrap), 0024 (pagination), 0025 (validation/serialization).

## Context

Transactions are the primary explorer entity for activity browsing. The list endpoint supports table-style browsing with slim DTOs. The detail endpoint supports both human-readable summaries and advanced/debugging views over the same resource, controlled by a query parameter.

### API Specification

**Location:** `apps/api/src/transactions/`

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

**List item fields (slim DTO):**

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
      "function_name": "swap",
      "human_readable": "Swapped 100 USDC for 95.2 XLM on Soroswap"
    }
  ],
  "operation_tree": [],
  "events": [
    {
      "event_type": "contract",
      "topics": [],
      "data": {},
      "interpretation": {
        "type": "swap",
        "human_readable": "Swapped 100 USDC for 95.2 XLM",
        "structured_data": {}
      }
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
      "human_readable": "Swapped 100 USDC for 95.2 XLM on Soroswap",
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

| Field             | Type           | Normal | Advanced | Description                              |
| ----------------- | -------------- | ------ | -------- | ---------------------------------------- |
| `hash`            | string         | yes    | yes      | Transaction hash                         |
| `ledger_sequence` | number         | yes    | yes      | Ledger sequence                          |
| `source_account`  | string         | yes    | yes      | Source account                           |
| `successful`      | boolean        | yes    | yes      | Success status                           |
| `fee_charged`     | number         | yes    | yes      | Fee in stroops                           |
| `result_code`     | string or null | yes    | yes      | Result code for failed txs               |
| `memo_type`       | string         | yes    | yes      | Memo type                                |
| `memo`            | string or null | yes    | yes      | Memo value                               |
| `created_at`      | string         | yes    | yes      | ISO timestamp                            |
| `operations`      | array          | yes    | yes      | Operations with human-readable summaries |
| `operation_tree`  | array          | yes    | yes      | Decoded invocation hierarchy             |
| `events`          | array          | yes    | yes      | Events with interpretations              |
| `envelope_xdr`    | string         | no     | yes      | Raw envelope XDR                         |
| `result_xdr`      | string         | no     | yes      | Raw result XDR                           |
| `parse_error`     | boolean        | yes    | yes      | Whether parse error occurred             |

**Important:** `result_meta_xdr` is NOT returned to the frontend. It is used server-side only for decode/validation. The `operation_tree` (decoded from `result_meta_xdr` at ingestion) is returned instead.

### Behavioral Requirements

- List responses optimized for table-style browsing (slim DTOs)
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

### Step 1: Module Scaffolding

Create `apps/api/src/transactions/` with module, controller, service, and DTOs.

### Step 2: List Endpoint

Implement `GET /transactions` with cursor pagination, filter parsing, and slim DTO response.

### Step 3: Detail Endpoint (Normal View)

Implement `GET /transactions/:hash` returning full detail with operations, operation_tree, events with interpretations, and result_code.

### Step 4: Advanced View

Add `?view=advanced` support to the detail endpoint, including envelope_xdr, result_xdr, raw parameters, and raw event payloads.

### Step 5: parse_error and Unknown Operation Handling

Ensure parse_error transactions are visible. Render unknown operation types as `{ type: 'unknown', raw_xdr: '...' }`.

### Step 6: Filter Implementation

Implement source_account, contract_id, and operation_type filters at the DB query level.

## Acceptance Criteria

- [ ] `GET /v1/transactions` returns paginated list with slim DTOs
- [ ] `GET /v1/transactions/:hash` returns full detail (normal view)
- [ ] `GET /v1/transactions/:hash?view=advanced` includes envelope_xdr, result_xdr, raw params
- [ ] result_meta_xdr never returned to frontend
- [ ] operation_tree returned from stored DB data (decoded at ingestion)
- [ ] Events include interpretations from event_interpretations table
- [ ] result_code present for failed transactions
- [ ] filter[source_account], filter[contract_id], filter[operation_type] work correctly
- [ ] parse_error transactions visible with null XDR-derived fields
- [ ] Unknown operations rendered as `{ type: 'unknown', raw_xdr: '...' }`
- [ ] Standard pagination envelope on list endpoint
- [ ] Appropriate error responses (400, 404, 500)

## Notes

- This is the most complex API module due to dual-mode detail views and the variety of data sources joined.
- The operation_tree is pre-computed at ingestion time; the API reads it from the DB, not from XDR decode.
- Event interpretations are joined from the event_interpretations table.
