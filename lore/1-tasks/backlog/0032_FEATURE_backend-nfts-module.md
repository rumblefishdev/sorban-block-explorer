---
id: '0032'
title: 'Backend: NFTs module (list + detail + transfers)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0024']
tags: [layer-backend, nfts, soroban]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: NFTs module (list + detail + transfers)

## Summary

Implement the NFTs module providing paginated NFT listing with collection/contract filters, NFT detail with sparse metadata tolerance, and NFT transfer history derived from Soroban events and linked transactions (not a separate table).

## Status: Backlog

**Current state:** Not started. Depends on tasks 0023 (bootstrap), 0024 (pagination).

## Context

NFTs on Stellar/Soroban are modeled as explorer entities with potentially sparse metadata. The ecosystem and metadata quality vary significantly, so responses must tolerate missing fields. Transfer history is derived from stored events and linked transactions rather than a dedicated NFT transfer table.

### API Specification

**Location:** `apps/api/src/nfts/`

---

#### GET /v1/nfts

**Method:** GET

**Path:** `/nfts`

**Query Parameters:**

| Parameter             | Type   | Default | Description               |
| --------------------- | ------ | ------- | ------------------------- |
| `limit`               | number | 20      | Items per page (max 100)  |
| `cursor`              | string | null    | Opaque pagination cursor  |
| `filter[collection]`  | string | null    | Filter by collection name |
| `filter[contract_id]` | string | null    | Filter by NFT contract ID |

**Response Shape (list):**

```json
{
  "data": [
    {
      "id": 1,
      "contract_id": "CCAB...DEF",
      "token_id": "42",
      "collection_name": "Stellar Punks",
      "owner_account": "GABC...XYZ",
      "name": "Punk #42",
      "media_url": "https://example.com/punk42.png"
    }
  ],
  "pagination": {
    "next_cursor": "eyJpZCI6Mn0=",
    "has_more": true
  }
}
```

---

#### GET /v1/nfts/:id

**Method:** GET

**Path:** `/nfts/:id`

**Path Parameters:**

| Parameter | Type   | Description     |
| --------- | ------ | --------------- |
| `id`      | number | Internal NFT ID |

**Response Shape:**

```json
{
  "id": 1,
  "contract_id": "CCAB...DEF",
  "token_id": "42",
  "collection_name": "Stellar Punks",
  "owner_account": "GABC...XYZ",
  "name": "Punk #42",
  "media_url": "https://example.com/punk42.png",
  "metadata": {
    "attributes": [{ "trait_type": "background", "value": "blue" }]
  },
  "minted_at_ledger": 10000000,
  "last_seen_ledger": 12345678
}
```

**Detail fields:**

| Field              | Type   | Nullable | Description                      |
| ------------------ | ------ | -------- | -------------------------------- |
| `id`               | number | no       | Internal NFT ID                  |
| `contract_id`      | string | no       | NFT contract ID                  |
| `token_id`         | string | no       | Token ID within the contract     |
| `collection_name`  | string | yes      | Collection name                  |
| `owner_account`    | string | yes      | Current owner account            |
| `name`             | string | yes      | NFT name                         |
| `media_url`        | string | yes      | Media/image URL                  |
| `metadata`         | object | yes      | Additional metadata (JSONB)      |
| `minted_at_ledger` | number | yes      | Ledger where NFT was minted      |
| `last_seen_ledger` | number | yes      | Most recent ledger with activity |

**Sparse metadata tolerance:** All fields except `id`, `contract_id`, and `token_id` may be null. The API must handle sparse metadata gracefully without errors.

---

#### GET /v1/nfts/:id/transfers

**Method:** GET

**Path:** `/nfts/:id/transfers`

**Path Parameters:**

| Parameter | Type   | Description     |
| --------- | ------ | --------------- |
| `id`      | number | Internal NFT ID |

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
      "from_account": "GABC...XYZ",
      "to_account": "GDEF...UVW",
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

**Transfer data source:** Derived from `soroban_events` + linked transactions, NOT a separate NFT transfers table. Transfer events are identified by matching contract_id and known transfer event patterns in the events table.

### Behavioral Requirements

- Sparse metadata tolerance: most fields nullable, no errors on missing metadata
- Transfers derived from soroban_events, not a dedicated table
- Filter by collection_name and contract_id
- NFT uniqueness scoped by contract_id + token_id

### Caching

| Endpoint                  | TTL     | Notes                              |
| ------------------------- | ------- | ---------------------------------- |
| `GET /nfts`               | 5-15s   | List may change as new NFTs appear |
| `GET /nfts/:id`           | 60-120s | NFT metadata changes infrequently  |
| `GET /nfts/:id/transfers` | 5-15s   | New transfers may appear           |

### Error Handling

- 400: Invalid id format, invalid filter values
- 404: NFT not found
- 500: Database errors

## Implementation Plan

### Step 1: Module Scaffolding

Create `apps/api/src/nfts/` with module, controller, service, and DTOs.

### Step 2: List Endpoint

Implement `GET /nfts` with cursor pagination and filter[collection]/filter[contract_id] support.

### Step 3: Detail Endpoint

Implement `GET /nfts/:id` with sparse metadata tolerance (nullable fields).

### Step 4: Transfers Endpoint

Implement `GET /nfts/:id/transfers` deriving transfer history from soroban_events table by matching contract_id and known transfer event patterns, joined with transactions for hash and timestamp.

## Acceptance Criteria

- [ ] `GET /v1/nfts` returns paginated NFT list
- [ ] `GET /v1/nfts/:id` returns NFT detail with all fields (nullable where sparse)
- [ ] `GET /v1/nfts/:id/transfers` returns paginated transfer history
- [ ] Transfers derived from soroban_events, not a separate table
- [ ] Sparse metadata handled gracefully (no errors on null fields)
- [ ] `filter[collection]` and `filter[contract_id]` work correctly
- [ ] Standard pagination and error envelopes
- [ ] 404 for non-existent NFTs

## Notes

- NFT metadata quality varies significantly across the Soroban ecosystem.
- Transfer derivation from events is the main implementation complexity.
- The contract_id + token_id unique constraint ensures correct NFT identity resolution.
