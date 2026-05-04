---
id: '0053'
title: 'Backend: Search module (unified search with query classification)'
type: FEATURE
status: completed
related_adr: ['0005', '0008', '0036']
related_tasks: ['0023', '0043', '0092']
tags: [layer-backend, search, full-text]
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
  - date: 2026-04-30
    status: active
    who: karolkow
    note: 'Promoted to active'
  - date: 2026-04-30
    status: active
    who: karolkow
    note: 'Reconciled spec with 22_get_search.sql + ADR 0036 (token→asset) + ADR 0008 envelope; added invalid_search_query / invalid_search_type codes; added per-group limit; narrowed response shape to (entity_type, identifier, label, surrogate_id)'
  - date: 2026-05-04
    status: completed
    who: karolkow
    note: >
      Implemented all 6 plan steps + 3 emerged decisions (base64 hash decoding, MAX_Q_LEN=256 defence-in-depth, graceful row mapping). 16 new unit tests + 1 OpenAPI test (222 total api tests passing). 5 files added (mod/dto/classifier/queries/handlers, ~899 lines). 4 files modified (lib.rs, main.rs, openapi/mod.rs, common/errors.rs, backend-overview.md §6.2 + §6.3). Search SQL is verbatim port of 22_get_search.sql; no DB schema changes. Two new canonical error codes (INVALID_SEARCH_QUERY, INVALID_SEARCH_TYPE) per ADR 0008.
---

# Backend: Search module (unified search with query classification)

## Summary

Implement the Search module providing unified search across all entity types with query classification, exact-match redirect behavior, and grouped broad-search results. Search uses prefix/exact matching on identifiers and full-text search via PostgreSQL GIN indexes on metadata.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Active

**Current state:** In progress. Dependencies complete: 0023 (bootstrap, superseded by Rust per ADR 0005), 0043 (pagination), 0092 (Rust API stack research). DB indexes used by `22_get_search.sql` already exist via migrations `0001_extensions`, `0002_identity_and_ledgers`, `0005_tokens_nfts`, `20260428000100_add_endpoint_query_indexes` — task 0133 not blocking.

## Context

Search is not a simple DB query wrapper. It is an API behavior surface that classifies input queries, supports exact-match redirect for unambiguous inputs, and returns grouped results for ambiguous queries. It spans all entity types in the explorer.

### API Specification

**Location:** `crates/api/src/search/`

---

#### GET /v1/search

**Method:** GET

**Path:** `/search`

**Query Parameters:**

| Parameter | Type   | Required | Description                                                                               |
| --------- | ------ | -------- | ----------------------------------------------------------------------------------------- |
| `q`       | string | yes      | Search query string                                                                       |
| `type`    | string | no       | Comma-separated type filter: `transaction`, `contract`, `asset`, `account`, `nft`, `pool` |
| `limit`   | int    | no       | Per-group result cap. Default 10, hard ceiling 50. Above 50 → 400.                        |

**Query Classification Rules:**

The classifier produces two derived inputs consumed by `22_get_search.sql`:

- `hash_bytes` (`BYTEA(32)`): non-NULL when `q` parses as 32-byte hex **or** base64. Drives both `transaction` (PK on `transaction_hash_index.hash`) and `pool` (PK on `liquidity_pools.pool_id`) exact-match branches — pool IDs are also 32-byte BYTEA.
- `strkey_prefix` (TEXT, upper-cased): non-NULL when `q` matches Stellar StrKey shape (full or prefix of `G…` / `C…`, base32 alphabet). Drives `account` (`idx_accounts_prefix`) and `contract` (`idx_contracts_prefix`) prefix branches.

Fully-typed `G…` / `C…` (56 chars, valid shape) and 64-hex-char queries SHOULD redirect at the route level (no broad search) when an entity exists.

`q` always feeds the trigram/FTS branches (asset code substring, NFT name substring, contract `search_vector`); no length/alphanumeric pre-filter applies — see `22_get_search.sql` CTEs.

**Response Shape (exact match / redirect):**

When the query unambiguously identifies a single entity:

```json
{
  "type": "redirect",
  "entity_type": "transaction",
  "entity_id": "7b2a8c1234567890abcdef..."
}
```

**Response Shape (broad search / grouped results):**

Narrow per-row shape — matches the `22_get_search.sql` projection. Each entity bucket carries the same four columns; rich entity payloads are NOT inlined (avoids fanning out joins across six entity types in a single endpoint).

```json
{
  "type": "results",
  "groups": {
    "transactions": [
      {
        "entity_type": "transaction",
        "identifier": "7b2a8c...",
        "label": "ledger 12345678",
        "surrogate_id": null
      }
    ],
    "accounts": [
      {
        "entity_type": "account",
        "identifier": "GABC...XYZ",
        "label": "stellar.org",
        "surrogate_id": 42
      }
    ],
    "assets": [
      {
        "entity_type": "asset",
        "identifier": "USDC",
        "label": "classic_credit",
        "surrogate_id": 1
      }
    ],
    "contracts": [
      {
        "entity_type": "contract",
        "identifier": "CCAB...DEF",
        "label": "Soroswap Router",
        "surrogate_id": 7
      }
    ],
    "nfts": [
      {
        "entity_type": "nft",
        "identifier": "Punk #42",
        "label": "CryptoPunks",
        "surrogate_id": 19
      }
    ],
    "pools": [
      {
        "entity_type": "pool",
        "identifier": "abcdef...",
        "label": "USDC / XLM",
        "surrogate_id": null
      }
    ]
  }
}
```

**Response fields:**

| Field          | Type        | Description                                                                                                                                     |
| -------------- | ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `type`         | string      | `redirect` for exact match, `results` for broad search                                                                                          |
| `entity_type`  | string      | (redirect) type of matched entity                                                                                                               |
| `entity_id`    | string      | (redirect) canonical id of matched entity                                                                                                       |
| `groups`       | object      | (results) entity-typed buckets, each an array of result rows                                                                                    |
| `identifier`   | string      | row-level: canonical human-shown id (hex for tx/pool, StrKey for account/contract, code for asset, name for NFT)                                |
| `label`        | string      | row-level: short context string (ledger seq, home_domain, asset_type, asset pair, etc.)                                                         |
| `surrogate_id` | int \| null | row-level: BIGINT FK used for routing on entities that have one (account, asset, contract, nft); `null` for tx/pool which route by `identifier` |

**Identifier-uniqueness caveats:**

- `asset.identifier` is the asset code — NOT unique (multiple issuers may share). Frontend MUST route `/assets/:id` via `surrogate_id`, not `identifier`.
- `nft.identifier` is the NFT name — NOT unique across collections. Frontend MUST route `/nfts/:id` via `surrogate_id`.
- `transaction.identifier` (hash) and `pool.identifier` (hex pool_id) ARE unique and route by `identifier`.

### Search Data Sources

Authoritative SQL: [`docs/architecture/database-schema/endpoint-queries/22_get_search.sql`](../../../docs/architecture/database-schema/endpoint-queries/22_get_search.sql).

| Entity          | Source Table             | Search Method                                                                                                                                                    | Index                                                |
| --------------- | ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| Transactions    | `transaction_hash_index` | Exact `hash = $hash_bytes` (32-byte BYTEA, hex/base64 input). Fires only when `hash_bytes` is non-NULL.                                                          | PK on `hash`                                         |
| Accounts        | `accounts`               | StrKey prefix `account_id LIKE $strkey_prefix \|\| '%'`. Fires only when `strkey_prefix` is non-NULL.                                                            | `idx_accounts_prefix`                                |
| Contracts       | `soroban_contracts`      | StrKey prefix when `strkey_prefix` non-NULL; otherwise `search_vector @@ plainto_tsquery('simple', $q)`.                                                         | `idx_contracts_prefix`, `idx_contracts_search` (GIN) |
| Assets          | `assets`                 | Trigram substring: `asset_code ILIKE '%' \|\| $q \|\| '%'`. Native XLM (`asset_type=0`, `asset_code IS NULL`) matched explicitly when `$q ILIKE 'xlm'/'native'`. | `idx_assets_code_trgm` (GIN gin_trgm_ops)            |
| NFTs            | `nfts`                   | Trigram substring on `name ILIKE '%' \|\| $q \|\| '%'`. NFT route param is the surrogate; `token_id` is NOT searched here.                                       | `idx_nfts_name_trgm` (GIN gin_trgm_ops)              |
| Liquidity Pools | `liquidity_pools`        | Exact `pool_id = $hash_bytes` (32-byte BYTEA — same shape as tx hash).                                                                                           | PK on `pool_id`                                      |

Full-text metadata search uses `soroban_contracts.search_vector` GIN index for contract name and metadata-driven queries when input is not a StrKey shape.

### Behavioral Requirements

- Query classification determines which `22_get_search.sql` CTE branches fire
- Exact match returns redirect response (frontend navigates directly)
- Ambiguous queries return grouped results
- Optional `type` filter maps to `:include_*` BOOLEAN flags on the SQL — disabled CTEs return zero rows; planner removes the branch
- Per-group cap via `limit` query param: default 10, hard ceiling 50; values >50 → 400
- Empty `q` parameter returns 400 error
- Full-text search uses PostgreSQL `tsvector`/`tsquery` via GIN index
- `identifier` for `asset` and `nft` is display-only and NOT unique; frontend routes via `surrogate_id`

### Caching

| Endpoint      | TTL      | Notes                                    |
| ------------- | -------- | ---------------------------------------- |
| `GET /search` | No cache | Variable params make caching impractical |

### Error Handling

Flat envelope per ADR 0008 — `{ code, message, details? }` (see `crates/api/src/common/errors.rs`). Two new canonical codes added by this task:

- `invalid_search_query` — `q` missing or empty
- `invalid_search_type` — unknown value in `type=...` filter
- `invalid_limit` (existing) — `limit` >50 or non-numeric
- `db_error` (existing) — 500 on DB failure

```json
{
  "code": "invalid_search_query",
  "message": "Search query 'q' parameter is required."
}
```

```json
{
  "code": "invalid_search_type",
  "message": "Invalid type filter. Allowed values: transaction, contract, asset, account, nft, pool"
}
```

## Implementation Plan

### Step 1: Route + handler setup

Create `crates/api/src/search/` with module, controller, service, and request/response types (ToSchema).

### Step 2: Query Classifier

Implement the query classification logic that determines entity type from query string patterns (hex detection, G/C prefix, length checks, alphanumeric checks).

### Step 3: Exact Match Resolution

Implement exact-match lookup for each entity type. If a classified query finds exactly one result, return a redirect response.

### Step 4: Broad Search

Implement grouped search across all (or filtered) entity types. Use prefix matching on identifiers and full-text search on metadata via GIN index.

### Step 5: Type Filter

Implement the optional `type` parameter that restricts search to specified entity types.

### Step 6: Full-Text Search Integration

Integrate with `soroban_contracts.search_vector` GIN index for metadata-driven search queries.

## Acceptance Criteria

- [x] `GET /v1/search?q=...` returns search results
- [x] Classifier produces `hash_bytes` (32-byte BYTEA from hex/base64) and `strkey_prefix` (upper-cased G…/C…) per `22_get_search.sql` contract
- [x] Exact match returns `{ type: 'redirect', entity_type, entity_id }`
- [x] Broad search returns `{ type: 'results', groups: {...} }` with narrow rows `(entity_type, identifier, label, surrogate_id)`
- [x] `type` filter maps to `:include_*` flags; unknown value → 400 `invalid_search_type`
- [x] Per-group cap honors `limit` (default 10, ceiling 50); >50 → 400 `invalid_limit`
- [x] Hits use the documented indexes: `transaction_hash_index PK`, `idx_accounts_prefix`, `idx_contracts_prefix`, `idx_contracts_search` (GIN), `idx_assets_code_trgm` (GIN), `idx_nfts_name_trgm` (GIN), `liquidity_pools PK`
- [x] Native XLM matched on `assets` via `asset_type=0` + `q ILIKE 'xlm'/'native'`
- [x] Empty `q` returns 400 `invalid_search_query`
- [x] No caching on search endpoint
- [x] Flat error envelope per ADR 0008
- [x] DTOs registered as `ToSchema` components in `openapi/mod.rs`
- [x] Docs updated: `docs/architecture/backend/backend-overview.md §6.3 Search` (response/limit details); database-schema-overview.md N/A — search indexes already documented

## Implementation Notes

**Module layout** — `crates/api/src/search/`:

| File            | Lines | Purpose                                                                                                                                                                                                                                     |
| --------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mod.rs`        | 26    | Router registration. `OpenApiRouter::new().routes(routes!(get_search))` mounted under `/v1`.                                                                                                                                                |
| `dto.rs`        | 87    | Wire types: `SearchResponse` discriminated union (`#[serde(tag="type")]` → `redirect` / `results`), `SearchHit` (4-col narrow row), `SearchGroups` (6 buckets), `EntityType` enum + `parse()`.                                              |
| `classifier.rs` | 172   | Pure function `classify(q) -> Classified { hash_bytes, strkey_prefix, is_fully_typed }`. Decodes 64-hex / 44-base64 / G-StrKey / C-StrKey shapes. 7 unit tests.                                                                             |
| `queries.rs`    | 310   | `fetch_redirect` (per-table sequential PK probes: tx → pool → account → contract) + `fetch_search` (verbatim port of `22_get_search.sql` with $1–$10 binds). Graceful row mapping (`filter_map` + log) — no panic on unknown `entity_type`. |
| `handlers.rs`   | 304   | Axum handler `get_search`. Validates `q` (non-empty + ≤256 bytes), `?type=` CSV, `?limit=` (1–50). Dispatches redirect vs broad. 10 unit tests covering all validation branches.                                                            |

**Wiring:**

- `lib.rs` + `main.rs` — `mod search;`
- `openapi/mod.rs` — `crate::search::router()` nested under `/v1`; 6 DTO types added to `components(schemas(...))`.
- `common/errors.rs` — added `INVALID_SEARCH_QUERY`, `INVALID_SEARCH_TYPE` constants.

**Docs:** `docs/architecture/backend/backend-overview.md` §6.2 inventory updated (`&limit=10` added) and §6.3 Search rewritten with classifier/redirect/results/limit details + canonical SQL link.

**Tests:** 16 new unit tests (7 classifier + 9 handler) + 1 OpenAPI registration test. Total 222 passing in `cargo test -p api` (was 220 before).

## Issues Encountered

- **Pre-push hook nx affected workspace-out-of-sync** when first push of feat branch had no upstream — fallback base resolved to `origin/master` (far behind develop). Workaround: `git branch --set-upstream-to=origin/develop` before first push so `@{upstream}` resolves correctly. Restored own upstream after success.
- **`git mv` + `Edit` ordering** during initial promote-task: `git mv` staged the rename of the original file content, then my `Edit` tool modified the file in working tree but those edits were unstaged. Required separate commit on develop to record the YAML history entry. Documented for future skill-flow.
- **clippy `result_large_err`** — handler validators returning `Result<T, axum::Response>` exceed clippy's 128-byte Err threshold. Added `#![allow(clippy::result_large_err)]` matching the convention from `common::extractors`.
- **clippy `assertions_on_constants`** — `MAX_Q_LEN` invariant test originally used runtime `assert!`; clippy preferred `const { ... }` block. Moved to `const _: () = { assert!(...) };`.

## Design Decisions

### From Plan

1. **Verbatim port of `22_get_search.sql` for `fetch_search`** — six narrow CTEs unioned, $1–$10 params, identical WHERE clauses and projections. Canonical SQL is the contract; Rust is the transport.

2. **Two-tier classifier output (`hash_bytes` + `strkey_prefix`)** — matches the SQL parameter contract directly; no per-entity dispatch logic in Rust to drift from SQL.

3. **`per_group_limit` default 10, ceiling 50** — per `22_get_search.sql` recommendation. 10 = dropdown UX size, 50 = DoS ceiling (6 CTE × 50 = 300 rows max).

4. **Two new error codes (`INVALID_SEARCH_QUERY`, `INVALID_SEARCH_TYPE`)** — chosen over reusing `INVALID_QUERY` / `INVALID_FILTER` to give frontend dropdown distinct UX branches without parsing message strings. ADR 0008 explicitly allows new codes; renaming would be breaking.

5. **Narrow 4-column row shape** — `(entity_type, identifier, label, surrogate_id)`. Frontend routes via `surrogate_id` for non-unique-display entities (asset, NFT). Caveats documented in DTO docstring + spec.

### Emerged

6. **Base64 (44-char) hash decoding alongside hex (64-char)** — Stellar tooling sometimes hands hashes around as base64 32-byte payloads. Plan only mentioned hex. Added with strict standard-alphabet validation (URL-safe rejected to avoid classifier ambiguity on short strings).

7. **`MAX_Q_LEN = 256` defence-in-depth cap** — not in original spec. Lambda payload limits eventually clip absurd inputs, but a tight cap rejects garbage at the request edge before any DB / regex work runs. Locked by const-block test (must stay 64 ≤ MAX_Q_LEN ≤ 1024).

8. **Graceful row mapping in `fetch_search` (`filter_map` + log instead of `expect()`)** — initial implementation used `.expect()` claiming "closed set" invariant. Audit flagged it as a panic potential if SQL ever drifts. Replaced with `filter_map` that logs unknown `entity_type` and skips the row, degrading to missing-bucket UX rather than 500.

9. **Sequential per-table redirect short-circuit (not consolidated UNION)** — initially proposed consolidating all 4 PK probes into one UNION ALL with `priority` ORDER BY for one-round-trip perf. Reverted to four sequential `fetch_optional` calls during review — clearer code path, easier to reason about per-table priority order, marginal latency gain (4 → 1 round-trip) not worth giving up the explicit early-return logic. Worst case is 2 round-trips (tx + pool) for hash-shape input, 1 for StrKey shape.

10. **Lowercase StrKey input normalisation** — plan didn't specify case handling. Implemented `to_ascii_uppercase()` before `is_strkey_prefix` check so `"gaaa"` works the same as `"GAAA"`. StrKey is base32-uppercase canonical; users may paste from sources that lowercase.

11. **`is_fully_typed` only for full 56-char StrKey, not prefix** — short prefix like `"GAB"` should NOT trigger redirect short-circuit (would make a useless `WHERE account_id = 'GAB'` exact lookup). Set `is_fully_typed = is_strkey_shape(...)` only when length is exactly 56.

## Future Work

- **Asset row label gap** — current canonical SQL projects `label = token_asset_type_name(asset_type)` ("classic_credit") for asset hits, which doesn't disambiguate multiple assets sharing the same code (e.g., 3× "USDC" from different issuers). Discuss with canonical SQL author about including issuer StrKey or asset name in label. → spawn follow-up task after design review.
- **Integration tests with real DB** — handler unit tests cover validation and routing; `fetch_redirect` and `fetch_search` are not exercised against real fixtures. Pattern in `tests_integration.rs` accepts `DATABASE_URL`-gated tests; add when fixture loader lands.
- **Asset/NFT search ranking** — current SQL returns rows in arbitrary order within each bucket (no ORDER BY). For high-traffic assets, surfacing the verified-issuer USDC ahead of spam should likely happen via off-chain whitelist or `holder_count DESC` heuristic.

## Notes

- Query classification is the core complexity of this module.
- The redirect vs results distinction enables the frontend to navigate directly on exact matches.
- Full-text search quality depends on the richness of indexed contract metadata.
- Asset trigram substring matching may produce false positives; broad search handles this gracefully.
