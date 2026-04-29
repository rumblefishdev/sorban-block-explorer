---
id: '0051'
title: 'Backend: NFTs module (list + detail + transfers)'
type: FEATURE
status: active
related_adr: ['0005', '0027', '0030', '0031']
related_tasks: ['0023', '0043', '0092']
tags: [layer-backend, nfts, soroban]
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
    note: 'Activated — bundled with 0052 (liquidity-pools) on shared branch, parallel module shape to 0048 accounts.'
  - date: 2026-04-29
    status: active
    who: karolkow
    note: 'Spec refresh vs current schema (ADR 0027/0030/0031): contract_id and owner_account are BIGINT FKs internally (soroban_contracts.id, accounts.id) — exposed as C/G-strkeys externally; transfers come from nft_ownership partitioned history table, not soroban_events.'
---

# Backend: NFTs module (list + detail + transfers)

## Summary

Implement the NFTs module providing paginated NFT listing with collection/contract filters, NFT detail with sparse metadata tolerance, and NFT transfer history derived from Soroban events and linked transactions (not a separate table).

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Backlog

**Current state:** Not started. Depends on tasks 0023 (bootstrap), 0043 (pagination).

## Context

NFTs on Stellar/Soroban are modeled as explorer entities with potentially sparse metadata. The ecosystem and metadata quality vary significantly, so responses must tolerate missing fields. Transfer history is derived from stored events and linked transactions rather than a dedicated NFT transfer table.

### API Specification

**Location:** `crates/api/src/nfts/`

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

**Storage note (ADR 0030/0031):** Internally `nfts.contract_id` is `BIGINT FK → soroban_contracts.id` and `nfts.current_owner_id` is `BIGINT FK → accounts.id`. Handlers JOIN to render the external C-strkey (`contract_id`) and G-strkey (`owner_account`) shapes shown above. `last_seen_ledger` in the response maps to `nfts.current_owner_ledger` (latest ledger where ownership state changed); the schema does not carry a separate "last seen" column.

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

**Transfer data source:** `nft_ownership` table (ADR 0027 §13) — partitioned ownership history filtered on `event_type = transfer` (NftEventType, ADR 0031), joined with `transactions` for hash/timestamp. Earlier draft of this task pointed at `soroban_events`; that was superseded once the dedicated `nft_ownership` table landed.

**`from_account` derivation:** `nft_ownership` stores only `owner_id` (the new owner after the event). `to_account` = current row's `owner_id`; `from_account` = previous row's `owner_id` for the same `nft_id`, obtained via `LAG(owner_id) OVER (PARTITION BY nft_id ORDER BY ledger_sequence, event_order)` (NULL on mint).

### Behavioral Requirements

- Sparse metadata tolerance: most fields nullable, no errors on missing metadata
- Transfers from `nft_ownership` (event_type = transfer), joined with transactions
- Filter by collection_name and contract_id (external C-strkey, resolved to internal FK)
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

### Step 1: Route + handler setup

Create `crates/api/src/nfts/` with module, controller, service, and request/response types (ToSchema).

### Step 2: List Endpoint

Implement `GET /nfts` with cursor pagination and filter[collection]/filter[contract_id] support.

### Step 3: Detail Endpoint

Implement `GET /nfts/:id` with sparse metadata tolerance (nullable fields).

### Step 4: Transfers Endpoint

Implement `GET /nfts/:id/transfers` reading from `nft_ownership` (filter `event_type = transfer`, NftEventType per ADR 0031), joined with `transactions` for hash/timestamp and with `accounts` to render `from_account`/`to_account` G-strkeys.

## Acceptance Criteria

- [ ] `GET /v1/nfts` returns paginated NFT list
- [ ] `GET /v1/nfts/:id` returns NFT detail with all fields (nullable where sparse)
- [ ] `GET /v1/nfts/:id/transfers` returns paginated transfer history
- [ ] Transfers sourced from `nft_ownership` (event_type = transfer), not soroban_events
- [ ] Sparse metadata handled gracefully (no errors on null fields)
- [ ] `filter[collection]` and `filter[contract_id]` work correctly
- [ ] Standard pagination and error envelopes
- [ ] 404 for non-existent NFTs

## Notes

- NFT metadata quality varies significantly across the Soroban ecosystem.
- Transfer derivation reads from `nft_ownership` (partitioned), filtered on NftEventType = transfer; main complexity is FK joins back to `accounts` and `transactions` to render external strkeys + tx hash.
- The contract_id + token_id unique constraint ensures correct NFT identity resolution.
