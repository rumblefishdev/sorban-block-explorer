---
id: '0053'
title: 'Backend: Search module (unified search with query classification)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0043']
tags: [layer-backend, search, full-text]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: Search module (unified search with query classification)

## Summary

Implement the Search module providing unified search across all entity types with query classification, exact-match redirect behavior, and grouped broad-search results. Search uses prefix/exact matching on identifiers and full-text search via PostgreSQL GIN indexes on metadata.

## Status: Backlog

**Current state:** Not started. Depends on tasks 0023 (bootstrap), 0043 (pagination).

## Context

Search is not a simple DB query wrapper. It is an API behavior surface that classifies input queries, supports exact-match redirect for unambiguous inputs, and returns grouped results for ambiguous queries. It spans all entity types in the explorer.

### API Specification

**Location:** `apps/api/src/search/`

---

#### GET /v1/search

**Method:** GET

**Path:** `/search`

**Query Parameters:**

| Parameter | Type   | Required | Description                                                                               |
| --------- | ------ | -------- | ----------------------------------------------------------------------------------------- |
| `q`       | string | yes      | Search query string                                                                       |
| `type`    | string | no       | Comma-separated type filter: `transaction`, `contract`, `token`, `account`, `nft`, `pool` |

**Query Classification Rules:**

| Pattern                  | Classification            | Entity Type |
| ------------------------ | ------------------------- | ----------- |
| 64-char hex string       | Transaction hash          | transaction |
| G + 56 chars             | Account ID                | account     |
| C + 56 chars             | Contract ID               | contract    |
| <= 12 alphanumeric chars | Asset code                | token       |
| 64-char non-hex string   | Pool ID                   | pool        |
| Anything else            | Full-text metadata search | multiple    |

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

When the query is ambiguous or matches multiple entities:

```json
{
  "type": "results",
  "groups": {
    "transactions": [
      {
        "hash": "7b2a8c...",
        "source_account": "GABC...XYZ",
        "created_at": "2026-03-20T12:00:00Z"
      }
    ],
    "accounts": [{ "account_id": "GABC...XYZ", "last_seen_ledger": 12345678 }],
    "tokens": [
      {
        "id": 1,
        "asset_code": "USDC",
        "asset_type": "classic",
        "name": "USD Coin"
      }
    ],
    "contracts": [
      { "contract_id": "CCAB...DEF", "contract_type": "dex", "metadata": {} }
    ],
    "nfts": [{ "id": 1, "name": "Punk #42", "contract_id": "CCAB...DEF" }],
    "pools": [
      { "pool_id": "abcdef...", "asset_a": {}, "asset_b": {}, "tvl": "1500000" }
    ]
  }
}
```

**Response fields:**

| Field         | Type   | Description                                            |
| ------------- | ------ | ------------------------------------------------------ |
| `type`        | string | `redirect` for exact match, `results` for broad search |
| `entity_type` | string | (redirect only) Type of matched entity                 |
| `entity_id`   | string | (redirect only) ID of matched entity                   |
| `groups`      | object | (results only) Grouped search results by entity type   |

### Search Data Sources

| Entity          | Source Table        | Search Method                                                     |
| --------------- | ------------------- | ----------------------------------------------------------------- |
| Transactions    | `transactions`      | Prefix/exact on `hash`                                            |
| Accounts        | `accounts`          | Prefix/exact on `account_id`                                      |
| Contracts       | `soroban_contracts` | Prefix/exact on `contract_id`, full-text on `search_vector` (GIN) |
| Tokens          | `tokens`            | Prefix/exact on `asset_code`                                      |
| NFTs            | `nfts`              | Prefix/exact on `token_id`, `name`                                |
| Liquidity Pools | `liquidity_pools`   | Prefix/exact on `pool_id`                                         |

Full-text metadata search uses the `soroban_contracts.search_vector` GIN index for contract name and metadata-driven queries.

### Behavioral Requirements

- Query classification determines search strategy
- Exact match returns redirect response (frontend navigates directly)
- Ambiguous queries return grouped results
- Optional `type` filter restricts search to specified entity types only
- Empty `q` parameter returns 400 error
- Full-text search uses PostgreSQL `tsvector`/`tsquery` via GIN index

### Caching

| Endpoint      | TTL      | Notes                                    |
| ------------- | -------- | ---------------------------------------- |
| `GET /search` | No cache | Variable params make caching impractical |

### Error Handling

- 400: Empty `q` parameter, invalid `type` filter values
- 500: Database errors

```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "Search query 'q' parameter is required."
  }
}
```

```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "Invalid type filter. Allowed values: transaction, contract, token, account, nft, pool"
  }
}
```

## Implementation Plan

### Step 1: Module Scaffolding

Create `apps/api/src/search/` with module, controller, service, and DTOs.

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

- [ ] `GET /v1/search?q=...` returns search results
- [ ] Query classification correctly identifies tx hashes, accounts, contracts, asset codes, pool IDs
- [ ] Exact match returns `{ type: 'redirect', entity_type, entity_id }`
- [ ] Broad search returns `{ type: 'results', groups: {...} }`
- [ ] Optional `type` filter restricts to specified entity types
- [ ] Full-text search uses GIN index on soroban_contracts.search_vector
- [ ] Prefix/exact matching on all identifier fields
- [ ] Empty `q` returns 400
- [ ] Invalid `type` filter returns 400
- [ ] No caching on search endpoint
- [ ] Standard error envelope

## Notes

- Query classification is the core complexity of this module.
- The redirect vs results distinction enables the frontend to navigate directly on exact matches.
- Full-text search quality depends on the richness of indexed contract metadata.
- Asset code matching (<=12 alphanumeric) may produce false positives; broad search handles this gracefully.
