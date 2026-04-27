---
id: '0050'
title: 'Backend: Contracts module (detail, interface, invocations, events)'
type: FEATURE
status: active
related_adr: ['0005', '0029', '0030', '0033', '0034']
related_tasks: ['0023', '0043', '0046', '0092', '0150']
tags: [layer-backend, contracts, soroban]
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
  - date: 2026-04-24
    status: active
    who: FilipDz
    note: 'Activated task.'
  - date: 2026-04-27
    status: active
    who: FilipDz
    note: >
      Implementation shipped under crates/api/src/contracts/ on branch
      feature/0050_backend-contracts-module — mirrors the transactions
      module layout (mod, dto, cursor, queries, handlers, plus a
      contract-only cache module). E10 (detail) + E11 (interface)
      hit Postgres only; E13 (invocations) + E14 (events) page over
      the appearance indexes (ADRs 0033 / 0034) and reconstruct
      per-node detail from public-archive XDR via
      stellar_archive::fetch_ledgers + xdr_parser::extract_invocations
      / extract_events (ADR 0029). Interface payload is read from
      wasm_interface_metadata.metadata (joined via wasm_hash) — not
      soroban_contracts.metadata as the original spec assumed
      (ADR 0023 split). 45 s ContractMetadataCache in AppState.
      collect_tx_metas promoted to pub in stellar_archive::extractors.
      OpenAPI registers all 4 routes + new schemas; api crate clippy
      clean (-D warnings) and 21/21 unit tests pass. Evergreen docs
      updated (backend-overview.md §4.1 / §8.1, wiki snapshot
      bootstrap status).
---

# Backend: Contracts module (detail, interface, invocations, events)

## Summary

Implement the Contracts module providing contract detail, public interface (function signatures), paginated invocation history, and paginated event history. This is the most Soroban-specific part of the API and the main place where indexed contract metadata and decoded usage history are exposed.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Backlog

**Current state:** Not started. Depends on tasks 0023 (bootstrap), 0043 (pagination).

## Context

Soroban contracts are first-class explorer entities. The contracts module exposes contract metadata, extracted interface signatures, invocation history, and event streams. Contract metadata is cached in Lambda in-memory cache (30-60s) for frequently accessed contracts.

### API Specification

**Location:** `crates/api/src/contracts/`

---

#### GET /v1/contracts/:contract_id

**Method:** GET

**Path:** `/contracts/:contract_id`

**Path Parameters:**

| Parameter     | Type   | Description              |
| ------------- | ------ | ------------------------ |
| `contract_id` | string | Contract ID (C+56 chars) |

**Response Shape:**

```json
{
  "contract_id": "CCAB...DEF",
  "wasm_hash": "abcdef1234...",
  "deployer_account": "GABC...XYZ",
  "deployed_at_ledger": 10000000,
  "contract_type": "dex",
  "is_sac": false,
  "metadata": {
    "name": "Soroswap DEX",
    "description": "Automated market maker"
  },
  "stats": {
    "invocation_count": 150000,
    "event_count": 300000
  }
}
```

**Detail fields:**

| Field                    | Type           | Description                              |
| ------------------------ | -------------- | ---------------------------------------- |
| `contract_id`            | string         | Contract ID (primary key)                |
| `wasm_hash`              | string or null | WASM hash                                |
| `deployer_account`       | string or null | Account that deployed the contract       |
| `deployed_at_ledger`     | number or null | Ledger where contract was deployed       |
| `contract_type`          | string         | Type: token, dex, lending, nft, other    |
| `is_sac`                 | boolean        | Whether this is a Stellar Asset Contract |
| `metadata`               | object or null | Explorer metadata (JSONB)                |
| `stats.invocation_count` | number         | Total invocations of this contract       |
| `stats.event_count`      | number         | Total events emitted by this contract    |

---

#### GET /v1/contracts/:contract_id/interface

**Method:** GET

**Path:** `/contracts/:contract_id/interface`

**Path Parameters:**

| Parameter     | Type   | Description              |
| ------------- | ------ | ------------------------ |
| `contract_id` | string | Contract ID (C+56 chars) |

**Response Shape:**

```json
{
  "functions": [
    {
      "name": "swap",
      "parameters": [
        { "name": "token_in", "type": "Address" },
        { "name": "token_out", "type": "Address" },
        { "name": "amount_in", "type": "i128" }
      ],
      "return_type": "i128"
    },
    {
      "name": "get_reserves",
      "parameters": [],
      "return_type": "Vec<i128>"
    }
  ]
}
```

**Interface data source:** Extracted from `soroban_contracts.metadata` at ingestion time (contract WASM interface extraction).

---

#### GET /v1/contracts/:contract_id/invocations

**Method:** GET

**Path:** `/contracts/:contract_id/invocations`

**Path Parameters:**

| Parameter     | Type   | Description              |
| ------------- | ------ | ------------------------ |
| `contract_id` | string | Contract ID (C+56 chars) |

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
      "transaction_hash": "7b2a8c...",
      "caller_account": "GABC...XYZ",
      "function_name": "swap",
      "function_args": [
        { "type": "Address", "value": "CCAB...DEF" },
        { "type": "i128", "value": "1000000" }
      ],
      "return_value": { "type": "i128", "value": "950000" },
      "successful": true,
      "ledger_sequence": 12345678,
      "created_at": "2026-03-20T12:00:00Z"
    }
  ],
  "pagination": {
    "next_cursor": "eyJpZCI6NDU2fQ==",
    "has_more": true
  }
}
```

**Invocation item fields:**

| Field              | Type           | Description                       |
| ------------------ | -------------- | --------------------------------- |
| `transaction_hash` | string         | Parent transaction hash           |
| `caller_account`   | string or null | Account that invoked the function |
| `function_name`    | string         | Invoked function name             |
| `function_args`    | array (JSONB)  | Decoded function arguments        |
| `return_value`     | object (JSONB) | Decoded return value              |
| `successful`       | boolean        | Whether invocation succeeded      |
| `ledger_sequence`  | number         | Ledger sequence                   |
| `created_at`       | string         | ISO 8601 timestamp                |

---

#### GET /v1/contracts/:contract_id/events

**Method:** GET

**Path:** `/contracts/:contract_id/events`

**Path Parameters:**

| Parameter     | Type   | Description              |
| ------------- | ------ | ------------------------ |
| `contract_id` | string | Contract ID (C+56 chars) |

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
      "transaction_hash": "7b2a8c...",
      "event_type": "contract",
      "topics": [{ "type": "Symbol", "value": "transfer" }],
      "data": {
        "type": "Map",
        "value": {}
      },
      "ledger_sequence": 12345678,
      "created_at": "2026-03-20T12:00:00Z"
    }
  ],
  "pagination": {
    "next_cursor": "eyJpZCI6Nzg5fQ==",
    "has_more": true
  }
}
```

**Event item fields:**

| Field              | Type           | Description                           |
| ------------------ | -------------- | ------------------------------------- |
| `transaction_hash` | string         | Parent transaction hash               |
| `event_type`       | string         | `contract`, `system`, or `diagnostic` |
| `topics`           | array (JSONB)  | Decoded event topics                  |
| `data`             | object (JSONB) | Decoded event data                    |
| `ledger_sequence`  | number         | Ledger sequence                       |
| `created_at`       | string         | ISO 8601 timestamp                    |

### Behavioral Requirements

- Contract metadata served from `soroban_contracts` table
- Interface data from `soroban_contracts.metadata` (extracted at ingestion)
- Invocations from `soroban_invocations` table, joined with transactions for hash
- Events from `soroban_events` table
- Stats (invocation_count, event_count) computed as aggregate counts
- Contract metadata cached in Lambda in-memory cache (30-60s)

### Caching

| Endpoint                                  | TTL     | Notes                                |
| ----------------------------------------- | ------- | ------------------------------------ |
| `GET /contracts/:contract_id`             | 60-120s | Metadata rarely changes              |
| `GET /contracts/:contract_id/interface`   | 60-120s | Interface is immutable once deployed |
| `GET /contracts/:contract_id/invocations` | 5-15s   | New invocations appear frequently    |
| `GET /contracts/:contract_id/events`      | 5-15s   | New events appear frequently         |

Lambda in-memory cache: 30-60s TTL for contract metadata of frequently accessed contracts.

### Error Handling

- 400: Invalid contract_id format
- 404: Contract not found
- 500: Database errors

## Implementation Plan

### Step 1: Route + handler setup

Create `crates/api/src/contracts/` with module, controller, service, and request/response types (ToSchema).

### Step 2: Contract Detail Endpoint

Implement `GET /contracts/:contract_id` querying `soroban_contracts` with aggregate stats from invocations and events tables.

### Step 3: Interface Endpoint

Implement `GET /contracts/:contract_id/interface` extracting function signatures from `soroban_contracts.metadata`.

### Step 4: Invocations Endpoint

Implement `GET /contracts/:contract_id/invocations` with cursor pagination from `soroban_invocations` table.

### Step 5: Events Endpoint

Implement `GET /contracts/:contract_id/events` with cursor pagination from `soroban_events` table.

### Step 6: In-Memory Caching

Implement Lambda in-memory cache for contract metadata with 30-60s TTL.

## Acceptance Criteria

- [x] `GET /v1/contracts/:contract_id` returns contract detail with stats
- [x] `GET /v1/contracts/:contract_id/interface` returns function signatures
- [x] `GET /v1/contracts/:contract_id/invocations` returns paginated invocation history
- [x] `GET /v1/contracts/:contract_id/events` returns paginated events
- [x] Stats include invocation_count and event_count (aggregated from the
      `*_appearances.amount` columns, since per-node rows no longer exist
      after ADRs 0033 / 0034)
- [x] Interface data sourced from indexed contract metadata — actual
      JSONB lives in `wasm_interface_metadata.metadata` (joined via
      `soroban_contracts.wasm_hash` per ADR 0023), not in
      `soroban_contracts.metadata` as the original spec assumed
- [x] Events queried from the indexer's contract-events table —
      `soroban_events_appearances` per ADR 0033 (table renamed and
      reduced to an appearance index since the spec was written;
      full topics/data fetched read-time from the public archive)
- [x] Contract metadata cached in Lambda in-memory cache (45 s,
      midpoint of the 30–60 s window) — `ContractMetadataCache`
- [x] Standard pagination and error envelopes on all paginated endpoints
- [x] 404 for non-existent contracts

## Notes

- This is the most Soroban-specific module and the richest in terms of sub-endpoints.
- Interface extraction happens at ingestion time; the API just reads from metadata.
