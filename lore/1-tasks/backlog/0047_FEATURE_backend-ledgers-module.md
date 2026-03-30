---
id: '0047'
title: 'Backend: Ledgers module (list + detail + linked transactions)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0043']
tags: [layer-backend, ledgers]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: Ledgers module (list + detail + linked transactions)

## Summary

Implement the Ledgers module providing paginated ledger listing and ledger detail with linked transactions. Closed ledgers are immutable and should be served with long-TTL cache headers. This is a straightforward historical/browsing module.

## Status: Backlog

**Current state:** Not started. Depends on tasks 0023 (bootstrap), 0043 (pagination).

## Context

Ledgers are the backbone of the explorer timeline. The list endpoint supports browsing recent ledgers in reverse sequence order. The detail endpoint includes linked transactions for a specific ledger. Since closed ledgers are immutable, aggressive caching is appropriate.

### API Specification

**Location:** `apps/api/src/ledgers/`

---

#### GET /v1/ledgers

**Method:** GET

**Path:** `/ledgers`

**Query Parameters:**

| Parameter | Type   | Default | Description              |
| --------- | ------ | ------- | ------------------------ |
| `limit`   | number | 20      | Items per page (max 100) |
| `cursor`  | string | null    | Opaque pagination cursor |

**Default ordering:** `sequence DESC`

**Response Shape (list):**

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
  "pagination": {
    "next_cursor": "eyJzZXEiOjEyMzQ1Njc3fQ==",
    "has_more": true
  }
}
```

---

#### GET /v1/ledgers/:sequence

**Method:** GET

**Path:** `/ledgers/:sequence`

**Path Parameters:**

| Parameter  | Type   | Description            |
| ---------- | ------ | ---------------------- |
| `sequence` | number | Ledger sequence number |

**Response Shape (detail):**

```json
{
  "sequence": 12345678,
  "hash": "abcdef...",
  "closed_at": "2026-03-20T12:00:00Z",
  "protocol_version": 21,
  "transaction_count": 150,
  "base_fee": 100,
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
    "pagination": {
      "next_cursor": "eyJpZCI6NDU2fQ==",
      "has_more": true
    }
  }
}
```

**Detail fields:**

| Field               | Type   | Description                                       |
| ------------------- | ------ | ------------------------------------------------- |
| `sequence`          | number | Ledger sequence number (primary key)              |
| `hash`              | string | Ledger hash (64-char hex)                         |
| `closed_at`         | string | ISO 8601 timestamp of ledger close                |
| `protocol_version`  | number | Protocol version at close                         |
| `transaction_count` | number | Number of transactions in this ledger             |
| `base_fee`          | number | Base fee in stroops                               |
| `transactions`      | object | Paginated list of linked transactions (slim DTOs) |

### Behavioral Requirements

- Default ordering is `sequence DESC` for list endpoint
- Linked transactions in detail use standard cursor pagination
- Closed ledgers are immutable: appropriate for long-TTL caching
- Only the newest ledger (or very recent ones) needs short TTL

### Caching

| Endpoint                 | Condition           | TTL   | Notes                             |
| ------------------------ | ------------------- | ----- | --------------------------------- |
| `GET /ledgers`           | --                  | 5-15s | List changes as new ledgers close |
| `GET /ledgers/:sequence` | closed (not latest) | 300s+ | Immutable, long cache             |
| `GET /ledgers/:sequence` | latest/recent       | 5-15s | May still be updating             |

### Error Handling

- 400: Invalid sequence format (non-numeric)
- 404: Ledger sequence not found
- 500: Database errors

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "Ledger with sequence 99999999 not found."
  }
}
```

## Implementation Plan

### Step 1: Module Scaffolding

Create `apps/api/src/ledgers/` with module, controller, service, and DTOs.

### Step 2: List Endpoint

Implement `GET /ledgers` with cursor pagination ordered by sequence DESC.

### Step 3: Detail Endpoint

Implement `GET /ledgers/:sequence` returning ledger fields plus paginated linked transactions.

### Step 4: Cache-Control Headers

Set long-TTL Cache-Control headers for closed/historical ledgers. Use short TTL for the most recent ledger sequence.

## Acceptance Criteria

- [ ] `GET /v1/ledgers` returns paginated list ordered by sequence DESC
- [ ] `GET /v1/ledgers/:sequence` returns ledger detail with linked transactions
- [ ] Linked transactions use standard cursor pagination envelope
- [ ] Long-TTL Cache-Control headers for closed ledgers
- [ ] Short-TTL for the most recent/latest ledger
- [ ] 404 for non-existent ledger sequences
- [ ] 400 for invalid sequence format
- [ ] Standard error envelope on all errors

## Notes

- Ledgers are one of the simplest modules since they have no filters and are immutable once closed.
- The linked transactions in the detail view reuse the slim transaction DTO from the transactions module.
- Cache-Control header logic needs to distinguish between historical and recent ledgers.
