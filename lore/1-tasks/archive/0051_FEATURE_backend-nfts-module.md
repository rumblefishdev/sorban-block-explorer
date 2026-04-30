---
id: '0051'
title: 'Backend: NFTs module (list + detail + transfers)'
type: FEATURE
status: completed
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
  - date: 2026-04-30
    status: completed
    who: karolkow
    note: >
      Implemented 3 endpoints (list/detail/transfers) bundled with task 0052 on
      shared branch. New module crates/api/src/nfts/ (~395 LOC). Wire shapes
      pinned to canonical SQL 15/16/17. Bug fix: LAG → LEAD for from_account
      window derivation (corrected canonical SQL too). Added filter[name]
      beyond task spec to match canonical input. New common helper
      `filters::reject_sql_wildcards_opt` (used here + retro-applied in
      assets module). 143 tests passing (+12 NFT integration tests).
---

# Backend: NFTs module (list + detail + transfers)

## Summary

Implement the NFTs module providing paginated NFT listing with collection/contract/name filters, NFT detail with sparse metadata tolerance, and NFT transfer history derived from the dedicated `nft_ownership` partitioned table joined with `transactions` for hash/timestamp.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Completed

**Completed:** 2026-04-30 (PR #152, bundled with task 0052). Dependencies on
0023 (bootstrap) and 0043 (pagination) were resolved before activation.

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

**Transfer data source:** `nft_ownership` table (ADR 0027 §13) — partitioned ownership history with **no `event_type` filter**, so the endpoint returns the full ownership timeline (mint, transfer, burn). The endpoint name `/transfers` is loose: it is really an "ownership-events" feed; each row carries `event_type_name` (`mint`/`transfer`/`burn`) so the frontend can render mint as "(mint)" with `from_account = NULL` and burn with `to_account = NULL`. Filtering to `event_type = transfer` would drop the mint row and break the LEAD-window derivation of `from_account` on the second-newest transfer (the row that immediately follows the dropped mint would lose its previous-owner reference). Earlier drafts of this task pointed at `soroban_events`; that was superseded once the dedicated `nft_ownership` table landed.

**`from_account` derivation:** `nft_ownership` stores only `owner_id` (the new owner after the event). `to_account` = current row's `owner_id`; `from_account` = the owner BEFORE this event = the owner AFTER the previous (older) event for the same `nft_id`. With the result set ordered DESC (newest first) the older event sits at the FOLLOWING window position, so the previous owner is `LEAD(owner_id) OVER (PARTITION BY nft_id ORDER BY created_at DESC, ledger_sequence DESC, event_order DESC)`. The mint row (oldest event, last in DESC window) yields `NULL` because LEAD has no following row — frontend renders that as "(mint)".

**`event_order` storage note:** `nft_ownership.event_order` is `SMALLINT NOT NULL` (ADR 0027 §13; `CHECK 0..=15` enforced by the schema). It is the per-`(nft_id, ledger_sequence)` ordinal that disambiguates multiple ownership events landing in the same ledger — required because two transfers of the same NFT can happen in a single ledger and `(created_at, ledger_sequence)` alone is not unique. It is monotonically increasing within an `(nft_id, ledger_sequence)` partition; NULLs are never expected (column is `NOT NULL`). The transfers cursor and the `LEAD` window function above both depend on this column for deterministic ordering — sorting on `(created_at, ledger_sequence)` alone would yield non-determinism on multi-event ledgers.

### Behavioral Requirements

- Sparse metadata tolerance: most fields nullable, no errors on missing metadata
- Transfers feed reads ALL ownership events from `nft_ownership` (mint, transfer, burn — no `event_type` filter), joined with transactions; each row surfaces its `event_type_name`
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

Implement `GET /nfts/:id/transfers` reading the full ownership timeline from `nft_ownership` (no `event_type` filter — mint/transfer/burn all surface), joined with `transactions` for hash/timestamp and with `accounts` to render `from_account`/`to_account` G-strkeys. Each row carries `event_type_name` (NftEventType per ADR 0031) so the frontend can label mint/transfer/burn distinctly.

## Acceptance Criteria

- [x] `GET /v1/nfts` returns paginated NFT list
- [x] `GET /v1/nfts/:id` returns NFT detail with all fields (nullable where sparse)
- [x] `GET /v1/nfts/:id/transfers` returns paginated transfer history
- [x] Transfers sourced from `nft_ownership` (full ownership timeline — mint/transfer/burn), not soroban_events; each row carries `event_type_name`
- [x] Sparse metadata handled gracefully (no errors on null fields)
- [x] `filter[collection]` and `filter[contract_id]` work correctly
- [x] `filter[name]` (substring trigram, rejects `%` / `_` literals) — added beyond original spec, matches canonical SQL `15_*.sql` `:name` input
- [x] Standard pagination and error envelopes
- [x] 404 for non-existent NFTs

## Implementation Notes

- New module `crates/api/src/nfts/` with `mod.rs`, `dto.rs`, `queries.rs`, `handlers.rs`. ~395 LOC.
- Three endpoints: `list_nfts`, `get_nft`, `list_nft_transfers`.
- Wire shapes pinned to canonical SQL `15/16/17_*.sql`.
- Cursors: `NftIdCursor { id: i32 }` (list) + `NftTransferCursor { created_at, ledger_sequence, event_order }` (transfers — matches `nft_ownership` PK exactly).
- Reused `common::*` helpers throughout: `Pagination<P>`, `cursor::encode`, `errors::*`, `filters::strkey_opt`, `filters::reject_sql_wildcards_opt` (new in this PR), `pagination::finalize_page` + `into_envelope`.
- Row types unified with wire DTOs (no separate `NftRow`/`NftTransferRow` — fields were 1:1 with `NftItem`/`NftTransferItem`, mappers were pure pass-through).
- Path-param `:id` parser extracted to file-local `parse_nft_id` (positive `i32`).

## Issues Encountered

- **Canonical SQL `17_get_nfts_transfers.sql` had a real bug**: used `LAG(owner_id)` for `from_account` derivation. With `ORDER BY ... DESC` (newest-first) the older event sits at the **following** window position, so `LAG` (preceding) yields the _next_ owner, not the previous. Fixed to `LEAD` in both canonical and `nfts/queries.rs`. Verified with concrete 3-row trace (Alice/Bob/Carol): `LAG` → 0/3 correct, `LEAD` → 3/3 correct.
- **Task spec said "filter `event_type = transfer`"** — but that would drop the mint row, breaking the LEAD-window derivation on the second-newest transfer (which would lose its previous-owner reference). Endpoint name `/transfers` is loose; it returns the full ownership timeline with `event_type_name` discriminator.

## Design Decisions

### From Plan

1. **Sparse metadata tolerance**: every nullable column maps to `Option<T>` in DTO. `metadata` JSONB → `Option<serde_json::Value>` per ADR 0037 §nfts.

2. **Transfer source = `nft_ownership` partitioned table** (per ADR 0027 §13), not `soroban_events`. Joined with `transactions` via composite FK `(transaction_id, created_at)` for partition-pruned reads.

### Emerged

3. **Added `filter[name]` to `GET /v1/nfts`**: task spec listed only `filter[collection]` + `filter[contract_id]`. Canonical SQL `15_*.sql:15-16` has `:name` as input $5 + `idx_nfts_name_trgm` GIN index. Implementing canonical fully = expose the param; otherwise the index is dead and the frontend §6.11 name-search use case is blocked.

4. **`/transfers` endpoint returns full ownership timeline** (mint/transfer/burn), not just transfers. Filtering to `event_type = transfer` would (a) hide the mint row from history view, (b) break `LEAD(owner_id)` derivation on the row that immediately follows mint. Endpoint name is loose; each row carries `event_type_name` so the frontend can label distinctly.

5. **`LEAD` instead of `LAG` for `from_account`**: canonical SQL had `LAG`; verification with a 3-row trace showed `LAG` returns wrong owner for every row in a DESC-ordered window. Corrected canonical SQL `17_*.sql` to match.

6. **No `Row` vs `Item` split**: shapes were 1:1, mappers were pure pass-through. Read straight into the wire DTO from sqlx — saves ~50 LOC of boilerplate. Module-level doc explains why this differs from `assets`/`liquidity_pools` (which DO have asymmetric Row vs Item).

7. **Pagination cursor `NftIdCursor { id: i32 }` (single field)**: `nfts.id` is SERIAL surrogate PK with no `created_at` column on `nfts`, so the project-default `TsIdCursor` does not fit. Single-key keyset on `n.id < $cursor` with order `n.id DESC` walks the PK index backward.

8. **`parse_nft_id` file-local helper**: 2 call sites (get + transfers), file-local rather than promoted to `common::path`. Per CLAUDE.md "three similar lines is better than premature abstraction" — promote when 3rd consumer appears.

## Notes

- NFT metadata quality varies significantly across the Soroban ecosystem.
- Transfer derivation reads the full ownership timeline from `nft_ownership` (partitioned, **no `event_type` filter** — mint/transfer/burn all surface). Each row carries `event_type_name` so the frontend can label distinctly. The `LEAD(owner_id)` window function reconstructs `from_account` and would break if mint were filtered out (the row immediately following mint would lose its previous-owner reference). Main complexity is FK joins back to `accounts` and `transactions` to render external G-strkeys + tx hash.
- The contract_id + token_id unique constraint ensures correct NFT identity resolution.
