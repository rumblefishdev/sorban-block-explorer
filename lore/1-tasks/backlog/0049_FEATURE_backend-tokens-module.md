---
id: '0049'
title: 'Backend: Tokens module (list + detail + transactions)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0043']
tags: [layer-backend, tokens, assets]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: Tokens module (list + detail + transactions)

## Summary

Implement the Tokens module providing paginated token listing with type/code filters, token detail, and token-related transaction history. The module must unify classic Stellar assets and Soroban token contracts through a single API while preserving identity distinctions between them.

## Status: Backlog

**Current state:** Not started. Depends on tasks 0023 (bootstrap), 0043 (pagination).

## Context

The explorer serves both classic Stellar assets and Soroban-based tokens through a unified token API. Classic assets are identified by `asset_code + issuer_address`, while Soroban tokens are identified by `contract_id`. The `:id` parameter must support both identification schemes.

### API Specification

**Location:** `apps/api/src/tokens/`

---

#### GET /v1/tokens

**Method:** GET

**Path:** `/tokens`

**Query Parameters:**

| Parameter      | Type   | Default | Description                             |
| -------------- | ------ | ------- | --------------------------------------- |
| `limit`        | number | 20      | Items per page (max 100)                |
| `cursor`       | string | null    | Opaque pagination cursor                |
| `filter[type]` | string | null    | Token type: `classic`, `sac`, `soroban` |
| `filter[code]` | string | null    | Filter by asset code                    |

**Response Shape (list):**

```json
{
  "data": [
    {
      "id": 1,
      "asset_type": "classic",
      "asset_code": "USDC",
      "issuer_address": "GCNY...ABC",
      "contract_id": null,
      "name": "USD Coin",
      "total_supply": "1000000.0000000",
      "holder_count": 5000
    }
  ],
  "pagination": {
    "next_cursor": "eyJpZCI6Mn0=",
    "has_more": true
  }
}
```

---

#### GET /v1/tokens/:id

**Method:** GET

**Path:** `/tokens/:id`

**Path Parameters:**

| Parameter | Type             | Description                                                                               |
| --------- | ---------------- | ----------------------------------------------------------------------------------------- |
| `id`      | string or number | Token identifier: numeric ID, or contract_id (C+56 chars), or asset_code+issuer composite |

**Response Shape:**

```json
{
  "id": 1,
  "asset_type": "classic",
  "asset_code": "USDC",
  "issuer_address": "GCNY...ABC",
  "contract_id": null,
  "name": "USD Coin",
  "total_supply": "1000000.0000000",
  "holder_count": 5000,
  "metadata": {
    "description": "A stablecoin pegged to USD",
    "icon_url": "https://example.com/usdc.png"
  }
}
```

**Detail fields:**

| Field            | Type           | Description                       |
| ---------------- | -------------- | --------------------------------- |
| `id`             | number         | Internal token ID                 |
| `asset_type`     | string         | `classic`, `sac`, or `soroban`    |
| `asset_code`     | string or null | Asset code (classic/SAC tokens)   |
| `issuer_address` | string or null | Issuer address (classic tokens)   |
| `contract_id`    | string or null | Contract ID (Soroban/SAC tokens)  |
| `name`           | string or null | Human-readable token name         |
| `total_supply`   | string or null | Total supply (numeric string)     |
| `holder_count`   | number         | Number of holders                 |
| `metadata`       | object or null | Additional token metadata (JSONB) |

---

#### GET /v1/tokens/:id/transactions

**Method:** GET

**Path:** `/tokens/:id/transactions`

**Path Parameters:**

| Parameter | Type             | Description      |
| --------- | ---------------- | ---------------- |
| `id`      | string or number | Token identifier |

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
      "operation_count": 1
    }
  ],
  "pagination": {
    "next_cursor": "eyJpZCI6MTIzfQ==",
    "has_more": true
  }
}
```

### Behavioral Requirements

- Token identity: classic = `asset_code + issuer_address`, Soroban = `contract_id`
- The `:id` param must support both identification schemes (numeric ID, contract_id, or code+issuer)
- Preserve distinction between classic and contract-based tokens
- Serve both through a unified API
- `filter[type]` accepts: `classic`, `sac`, `soroban`
- `filter[code]` matches against `asset_code`

### Caching

| Endpoint                       | TTL     | Notes                                |
| ------------------------------ | ------- | ------------------------------------ |
| `GET /tokens`                  | 5-15s   | List may change as new tokens appear |
| `GET /tokens/:id`              | 60-120s | Token metadata changes infrequently  |
| `GET /tokens/:id/transactions` | 5-15s   | New transactions may appear          |

### Error Handling

- 400: Invalid filter[type] value, invalid id format
- 404: Token not found
- 500: Database errors

## Implementation Plan

### Step 1: Module Scaffolding

Create `apps/api/src/tokens/` with module, controller, service, and DTOs.

### Step 2: Token ID Resolution

Implement ID resolution logic that determines whether `:id` is a numeric ID, a contract_id (C+56), or a code+issuer composite, and queries accordingly.

### Step 3: List Endpoint

Implement `GET /tokens` with cursor pagination and filter[type]/filter[code] support.

### Step 4: Detail Endpoint

Implement `GET /tokens/:id` with the multi-scheme ID resolution.

### Step 5: Token Transactions Endpoint

Implement `GET /tokens/:id/transactions` with cursor pagination. Join through operations/events to find transactions involving this token.

## Acceptance Criteria

- [ ] `GET /v1/tokens` returns paginated token list
- [ ] `GET /v1/tokens/:id` returns token detail
- [ ] `GET /v1/tokens/:id/transactions` returns paginated transaction list
- [ ] `:id` supports numeric ID, contract_id, and code+issuer identification
- [ ] `filter[type]` works for classic, sac, soroban
- [ ] `filter[code]` filters by asset_code
- [ ] Classic and Soroban tokens served through unified API
- [ ] Identity distinctions preserved (asset_code+issuer vs contract_id)
- [ ] Standard pagination and error envelopes
- [ ] 404 for non-existent tokens

## Notes

- The multi-scheme ID resolution is the main complexity in this module.
- Token transactions may require joining through operations or events depending on token type.
- SAC (Stellar Asset Contract) tokens bridge classic and Soroban; they have both asset_code/issuer and contract_id.
