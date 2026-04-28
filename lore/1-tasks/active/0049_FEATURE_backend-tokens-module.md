---
id: '0049'
title: 'Backend: Assets module (list + detail + transactions)'
type: FEATURE
status: active
related_adr: ['0005', '0036', '0037']
related_tasks: ['0023', '0043', '0050', '0092']
tags: [layer-backend, assets]
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
  - date: 2026-04-23
    status: backlog
    who: karolkow
    note: 'Updated per task 0154: tokens→assets rename, filter values updated (classic→classic_credit, added native)'
  - date: 2026-04-27
    status: active
    who: FilipDz
    note: >
      Activated task. Picking up after task 0050 (Contracts module) shipped —
      same M2 list/detail/sub-resource shape, mirrors the contracts module
      layout in crates/api/src/. Will adapt to task 0043 pagination helpers
      once Karol's PR #124 lands.
  - date: 2026-04-28
    status: active
    who: FilipDz
    note: >
      Implementation shipped under crates/api/src/assets/ on branch
      feature/0049_backend-assets-module. Mirrors the transactions
      post-refactor layout (mod / dto / queries / handlers, no per-module
      cursor) and reuses common::* throughout. List paginates by `id DESC`
      via custom AssetIdCursor (assets is unpartitioned + has no
      created_at). :id resolution tries numeric → C-StrKey → code-issuer
      composite. /transactions sub-resource composes per-asset_type
      predicates against operations_appearances; native XLM short-circuits
      to empty page. SEP-1 description/home_page emitted as null pending
      task 0164.
  - date: 2026-04-28
    status: active
    who: FilipDz
    note: >
      Aligned response shape and SQL with the canonical SQL deliverable
      from task 0167 (docs/architecture/database-schema/endpoint-queries/
      08_get_assets_list.sql, 09_get_assets_by_id.sql,
      10_get_assets_transactions.sql) BEFORE first PR review — caught
      the same divergence pattern that hit 0050 post-merge. Wire-shape
      changes vs initial 0049 implementation: rename `issuer_address` →
      `issuer`; `asset_type` is now the raw SMALLINT (i16) and decoded
      label moves to `asset_type_name: Option<String>` (via
      `token_asset_type_name()` SQL helper, ADR 0031); detail response
      adds `deployed_at_ledger` from `soroban_contracts.deployed_at_ledger`;
      `/transactions` rows add `has_soroban` and `operation_types[]`
      (LATERAL `array_agg(DISTINCT op_type_name(...))`); `filter[code]`
      switched from exact match to substring trigram (`ILIKE '%' || $1 || '%'`)
      served by `idx_assets_code_trgm` (gin_trgm_ops). Two architectural
      divergences kept deliberately and documented in queries.rs module
      docs: (a) `:id` resolution stays at the API layer (3 fetch_by_*
      paths, no surrogate-first single-SQL) and (b) `/transactions` is
      one OR'd query covering both classic and contract identity branches
      instead of canonical's split A/B statements — both produce the
      same result. 78/78 tests green; clippy clean (-D warnings).
---

# Backend: Assets module (list + detail + transactions)

## Summary

Implement the Assets module providing paginated asset listing with type/code filters, asset detail, and asset-related transaction history. The module must unify native XLM, classic credit assets, SACs, and Soroban-native tokens through a single API while preserving identity distinctions between them.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Backlog

**Current state:** Not started. Depends on tasks 0023 (bootstrap), 0043 (pagination).

## Context

The explorer serves all Stellar asset classes through a unified asset API. Classic credit assets are identified by `asset_code + issuer_address`, Soroban tokens by `contract_id`. The `:id` parameter must support both identification schemes.

### API Specification

**Location:** `crates/api/src/assets/`

---

#### GET /v1/assets

**Method:** GET

**Path:** `/assets`

**Query Parameters:**

| Parameter      | Type   | Default | Description                                              |
| -------------- | ------ | ------- | -------------------------------------------------------- |
| `limit`        | number | 20      | Items per page (max 100)                                 |
| `cursor`       | string | null    | Opaque pagination cursor                                 |
| `filter[type]` | string | null    | Asset type: `native`, `classic_credit`, `sac`, `soroban` |
| `filter[code]` | string | null    | Filter by asset code                                     |

**Response Shape (list):**

```json
{
  "data": [
    {
      "id": 1,
      "asset_type": "classic_credit",
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

#### GET /v1/assets/:id

**Method:** GET

**Path:** `/assets/:id`

**Path Parameters:**

| Parameter | Type             | Description                                                                               |
| --------- | ---------------- | ----------------------------------------------------------------------------------------- |
| `id`      | string or number | Asset identifier: numeric ID, or contract_id (C+56 chars), or asset_code+issuer composite |

**Response Shape:**

```json
{
  "id": 1,
  "asset_type": "classic_credit",
  "asset_code": "USDC",
  "issuer_address": "GCNY...ABC",
  "contract_id": null,
  "name": "USD Coin",
  "total_supply": "1000000.0000000",
  "holder_count": 5000,
  "description": "A stablecoin pegged to USD",
  "icon_url": "https://example.com/usdc.png",
  "home_page": "https://centre.io"
}
```

**Detail fields:**

| Field            | Type           | Description                                     |
| ---------------- | -------------- | ----------------------------------------------- |
| `id`             | number         | Internal asset ID                               |
| `asset_type`     | string         | `native`, `classic_credit`, `sac`, or `soroban` |
| `asset_code`     | string or null | Asset code (classic_credit/SAC assets)          |
| `issuer_address` | string or null | Issuer address (classic_credit assets)          |
| `contract_id`    | string or null | Contract ID (Soroban/SAC assets)                |
| `name`           | string or null | Human-readable asset name                       |
| `total_supply`   | string or null | Total supply (numeric string)                   |
| `holder_count`   | number         | Number of holders                               |
| `description`    | string or null | Asset description (SEP-1 metadata)              |
| `icon_url`       | string or null | Icon URL (SEP-1 metadata)                       |
| `home_page`      | string or null | Home page URL (SEP-1 metadata)                  |

---

#### GET /v1/assets/:id/transactions

**Method:** GET

**Path:** `/assets/:id/transactions`

**Path Parameters:**

| Parameter | Type             | Description      |
| --------- | ---------------- | ---------------- |
| `id`      | string or number | Asset identifier |

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

- Asset identity: classic_credit = `asset_code + issuer_address`, Soroban = `contract_id`, native = singleton
- The `:id` param must support both identification schemes (numeric ID, contract_id, or code+issuer)
- Preserve distinction between native, classic credit, SAC, and Soroban-native assets
- Serve all through a unified API
- `filter[type]` accepts: `native`, `classic_credit`, `sac`, `soroban`
- `filter[code]` matches against `asset_code`

### Caching

| Endpoint                       | TTL     | Notes                                |
| ------------------------------ | ------- | ------------------------------------ |
| `GET /assets`                  | 5-15s   | List may change as new assets appear |
| `GET /assets/:id`              | 60-120s | Asset metadata changes infrequently  |
| `GET /assets/:id/transactions` | 5-15s   | New transactions may appear          |

### Error Handling

- 400: Invalid filter[type] value, invalid id format
- 404: Asset not found
- 500: Database errors

## Implementation Plan

### Step 1: Route + handler setup

Create `crates/api/src/assets/` with module, controller, service, and request/response types (ToSchema).

### Step 2: Asset ID Resolution

Implement ID resolution logic that determines whether `:id` is a numeric ID, a contract_id (C+56), or a code+issuer composite, and queries accordingly.

### Step 3: List Endpoint

Implement `GET /assets` with cursor pagination and filter[type]/filter[code] support.

### Step 4: Detail Endpoint

Implement `GET /assets/:id` with the multi-scheme ID resolution.

### Step 5: Asset Transactions Endpoint

Implement `GET /assets/:id/transactions` with cursor pagination. Join through operations/events to find transactions involving this asset.

## Acceptance Criteria

- [x] `GET /v1/assets` returns paginated asset list
- [x] `GET /v1/assets/:id` returns asset detail
- [x] `GET /v1/assets/:id/transactions` returns paginated transaction list
- [x] `:id` supports numeric ID, contract_id, and code+issuer identification
      (priority order: numeric → C-StrKey → code-issuer; first that parses
      drives the SQL lookup)
- [x] `filter[type]` works for native, classic_credit, sac, soroban
      (`domain::TokenAssetType` via `common::filters::parse_enum_opt`)
- [x] `filter[code]` filters by `asset_code` (exact match)
- [x] All asset classes served through unified API
- [x] Identity distinctions preserved per asset_type. Native XLM short-circuits
      to empty page on the `/transactions` sub-resource (no
      `operations_appearances` rows reference it; documented via
      `asset_predicate_present`)
- [x] Standard pagination (`Paginated<T>` + `PageInfo` from
      `openapi::schemas` via `common::pagination::into_envelope`) and
      canonical error envelopes (`common::errors::*`)
- [x] 404 for non-existent assets — covered by integration test
      `assets_detail_unknown_id_returns_404_against_real_db`

> SEP-1 `description` / `home_page` are emitted as `null` (not implemented).
> Per ADR 0037 §342 they live in S3 per-entity, not in the DB; their
> hydration is owned by task 0164 and is a deferred follow-up to this PR.

## Notes

- The multi-scheme ID resolution is the main complexity in this module.
- Asset transactions may require joining through operations or events depending on asset type.
- SAC (Stellar Asset Contract) assets bridge classic and Soroban; they have both asset_code/issuer and contract_id.
