---
id: '0048'
title: 'Backend: Accounts module (detail + balances + transactions)'
type: FEATURE
status: completed
related_adr: ['0005']
related_tasks: ['0023', '0043', '0092']
tags: [layer-backend, accounts]
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
    who: FilipDz
    note: 'Activated — natural continuation of 0049 (tokens) + 0050 (contracts) backend modules.'
  - date: 2026-05-05
    status: completed
    who: FilipDz
    note: 'Shipped per canonical SQL 06/07. 11 integration tests, OpenAPI regenerated.'
---

# Backend: Accounts module (detail + balances + transactions)

## Summary

Implement the Accounts module providing account detail (summary + balances) and account-related transaction history. The account scope is intentionally limited to summary, balances, and transactions only. Do NOT add operations, effects, or offers unless the architecture is explicitly updated.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Completed

**Current state:** Shipped 2026-05-05 (FilipDz). 11 integration tests
passing, OpenAPI regenerated. See history entry above.

## Context

Accounts are a core explorer entity but with intentionally limited scope in the current architecture. The module provides account summary with current balances and a paginated transaction history. This keeps the backend contract aligned with what the frontend is expected to show.

### API Specification

**Location:** `crates/api/src/accounts/`

---

#### GET /v1/accounts/:account_id

**Method:** GET

**Path:** `/accounts/:account_id`

**Path Parameters:**

| Parameter    | Type   | Description                     |
| ------------ | ------ | ------------------------------- |
| `account_id` | string | Stellar account ID (G+56 chars) |

**Response Shape:**

```json
{
  "account_id": "GABC...XYZ",
  "sequence_number": 123456789,
  "balances": [
    {
      "asset_type": "native",
      "balance": "1000.0000000"
    },
    {
      "asset_type": "credit_alphanum4",
      "asset_code": "USDC",
      "asset_issuer": "GCNY...ABC",
      "balance": "500.0000000"
    }
  ],
  "home_domain": "example.com",
  "first_seen_ledger": 10000000,
  "last_seen_ledger": 12345678
}
```

**Detail fields:**

| Field               | Type           | Description                           |
| ------------------- | -------------- | ------------------------------------- |
| `account_id`        | string         | Stellar account ID                    |
| `sequence_number`   | number         | Current sequence number               |
| `balances`          | array (JSONB)  | Array of balance objects              |
| `home_domain`       | string or null | Account home domain                   |
| `first_seen_ledger` | number         | First ledger this account appeared in |
| `last_seen_ledger`  | number         | Most recent ledger with activity      |

---

#### GET /v1/accounts/:account_id/transactions

**Method:** GET

**Path:** `/accounts/:account_id/transactions`

**Path Parameters:**

| Parameter    | Type   | Description                     |
| ------------ | ------ | ------------------------------- |
| `account_id` | string | Stellar account ID (G+56 chars) |

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
      "ledger_sequence": 12345678,
      "source_account": "GABC...XYZ",
      "successful": true,
      "fee_charged": 100,
      "created_at": "2026-03-20T12:00:00Z",
      "operation_count": 3,
      "memo_type": "text",
      "memo": "payment"
    }
  ],
  "pagination": {
    "next_cursor": "eyJpZCI6Nzg5fQ==",
    "has_more": true
  }
}
```

### Behavioral Requirements

- Account scope is intentionally limited: summary + balances + transactions ONLY
- Do NOT add operations, effects, or offers unless architecture is explicitly updated
- Balances are stored as JSONB array in the accounts table
- Transaction history filtered by source_account matching the account_id
- Standard cursor pagination on transaction list

### Caching

| Endpoint                                 | TTL   | Notes                                     |
| ---------------------------------------- | ----- | ----------------------------------------- |
| `GET /accounts/:account_id`              | 5-15s | Account state may update with new ledgers |
| `GET /accounts/:account_id/transactions` | 5-15s | New transactions may appear               |

### Error Handling

- 400: Invalid account_id format (not G+56 chars)
- 404: Account not found in database
- 500: Database errors

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "Account 'GABC...XYZ' not found."
  }
}
```

## Implementation Plan

### Step 1: Route + handler setup

Create `crates/api/src/accounts/` with module, controller, service, and request/response types (ToSchema).

### Step 2: Account Detail Endpoint

Implement `GET /accounts/:account_id` querying the `accounts` table. Return account_id, sequence_number, balances (JSONB), home_domain, first_seen_ledger, last_seen_ledger.

### Step 3: Account Transactions Endpoint

Implement `GET /accounts/:account_id/transactions` querying the `transactions` table filtered by `source_account = :account_id` with standard cursor pagination.

### Step 4: Validation

Validate account_id format (G+56 characters). Return 400 for malformed IDs.

## Acceptance Criteria

- [x] `GET /v1/accounts/:account_id` returns account detail with balances
- [x] `GET /v1/accounts/:account_id/transactions` returns paginated transaction list
- [x] Balances served from `account_balances_current` (per ADR 0026 / ADR 0035 — original spec said "JSONB array on accounts" but that table never existed; canonical SQL 06 is authoritative)
- [x] Transaction list uses standard cursor pagination envelope (`Paginated<T>` + `TsIdCursor`)
- [x] Account_id validated as G+56 chars format (`path::strkey('G', "account_id")`)
- [x] 404 for non-existent accounts (both detail and transactions sub-resource)
- [x] 400 for invalid account_id format
- [x] No operations, effects, or offers endpoints present
- [x] Standard error envelope on all errors

## Notes

- The limited scope is a deliberate architectural choice, not an oversight.
- Balances are stored as JSONB and served as-is; no server-side balance computation.
- Transaction filtering by source_account uses the existing index on `transactions.source_account`.
