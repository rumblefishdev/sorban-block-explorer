---
id: '0055'
title: 'Backend: API Gateway response caching and cache-control headers'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023']
tags: [layer-backend, caching, api-gateway, performance]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: API Gateway response caching and cache-control headers

## Summary

Define and implement the per-endpoint Cache-Control header strategy for API Gateway response caching. This includes long TTLs for immutable resources (finalized transactions, closed ledgers), short TTLs for frequently changing data (lists, network stats), medium TTLs for slowly changing data (contract metadata), and no cache for variable-parameter endpoints (search).

## Status: Backlog

**Current state:** Not started. Depends on task 0023 (NestJS API bootstrap).

## Context

Caching at the API Gateway level is the primary response caching mechanism. CloudFront is NOT used for the API in the initial topology. Cache keys must include the full path plus all query parameters so that different filters produce different cache entries.

### API Specification

**Location:** Cache-Control headers set in NestJS controllers/interceptors. API Gateway stage-level caching configuration.

### Per-Endpoint Cache Mapping

#### Long TTL (immutable, 300s+)

| Endpoint                                     | TTL   | Rationale                            |
| -------------------------------------------- | ----- | ------------------------------------ |
| `GET /ledgers/:sequence` (closed/historical) | 300s+ | Closed ledgers are immutable         |
| `GET /transactions/:hash` (finalized)        | 300s+ | Finalized transactions are immutable |

#### Short TTL (5-15s)

| Endpoint                                  | TTL   | Rationale                              |
| ----------------------------------------- | ----- | -------------------------------------- |
| `GET /network/stats`                      | 5-15s | Near-real-time summary data            |
| `GET /transactions` (list)                | 5-15s | New transactions appear frequently     |
| `GET /ledgers` (list)                     | 5-15s | New ledgers close frequently           |
| `GET /accounts/:account_id`               | 5-15s | Account state updates with new ledgers |
| `GET /accounts/:account_id/transactions`  | 5-15s | New transactions may appear            |
| `GET /tokens` (list)                      | 5-15s | Token list may update                  |
| `GET /tokens/:id/transactions`            | 5-15s | New transactions                       |
| `GET /nfts` (list)                        | 5-15s | New NFTs may appear                    |
| `GET /nfts/:id/transfers`                 | 5-15s | New transfers may appear               |
| `GET /liquidity-pools` (list)             | 5-15s | Pool state changes frequently          |
| `GET /liquidity-pools/:id`                | 5-15s | Pool reserves update                   |
| `GET /liquidity-pools/:id/transactions`   | 5-15s | New pool transactions                  |
| `GET /contracts/:contract_id/invocations` | 5-15s | New invocations                        |
| `GET /contracts/:contract_id/events`      | 5-15s | New events                             |

#### Medium TTL (60-120s)

| Endpoint                                | TTL     | Rationale                             |
| --------------------------------------- | ------- | ------------------------------------- |
| `GET /contracts/:contract_id`           | 60-120s | Contract metadata rarely changes      |
| `GET /contracts/:contract_id/interface` | 60-120s | Interface is immutable once deployed  |
| `GET /tokens/:id`                       | 60-120s | Token metadata changes infrequently   |
| `GET /nfts/:id`                         | 60-120s | NFT metadata changes infrequently     |
| `GET /liquidity-pools/:id/chart`        | 60-120s | Snapshot data updates less frequently |

#### No Cache

| Endpoint      | Rationale                                      |
| ------------- | ---------------------------------------------- |
| `GET /search` | Variable query params make caching impractical |

### Cache-Control Header Format

```
Cache-Control: public, max-age=300
```

For no-cache endpoints:

```
Cache-Control: no-store
```

### Cache Key Requirements

- Cache keys MUST include full path + all query parameters
- Different filters produce different cache entries
- Example: `/transactions?filter[source_account]=GABC` and `/transactions?filter[contract_id]=CCAB` are separate cache entries
- Cursor values produce unique cache entries (each page is cached independently)

### Response Shape

No separate response for caching. Cache-Control is a response header added to existing endpoint responses.

```
HTTP/1.1 200 OK
Cache-Control: public, max-age=300
Content-Type: application/json

{ ... normal response body ... }
```

### Behavioral Requirements

- CloudFront NOT used for API in initial topology
- Caching at API Gateway stage level only
- Cache keys include full path + all query parameters
- NestJS sets appropriate Cache-Control headers per endpoint
- API Gateway respects Cache-Control headers for its stage cache
- Immutable resource detection: closed ledgers (not the latest), finalized transactions

### Error Handling

- Error responses (4xx, 5xx) should NOT be cached
- Cache-Control headers only set on successful (2xx) responses

## Implementation Plan

### Step 1: Cache-Control Interceptor

Create a NestJS interceptor or decorator system that sets Cache-Control headers based on endpoint configuration. Each controller method specifies its cache tier (long, short, medium, none).

### Step 2: Immutable Resource Detection

For `GET /ledgers/:sequence`, determine if the ledger is a closed historical ledger (long TTL) vs the latest/recent ledger (short TTL). For `GET /transactions/:hash`, finalized transactions get long TTL.

### Step 3: Per-Endpoint Configuration

Apply cache tier to each endpoint controller as documented in the cache mapping table.

### Step 4: Error Response Exclusion

Ensure error responses (4xx, 5xx) include `Cache-Control: no-store` or omit caching headers.

### Step 5: API Gateway Configuration Documentation

Document the API Gateway stage-level cache configuration needed to respect the Cache-Control headers set by NestJS.

## Acceptance Criteria

- [ ] Cache-Control headers set correctly per endpoint
- [ ] Long TTL (300s+) for closed ledgers and finalized transactions
- [ ] Short TTL (5-15s) for lists, stats, and frequently changing detail
- [ ] Medium TTL (60-120s) for contract/token/NFT metadata
- [ ] No cache for search endpoint
- [ ] Cache keys include full path + all query parameters
- [ ] Error responses not cached
- [ ] CloudFront not used for API (confirmed)
- [ ] Immutable detection logic for ledgers and transactions
- [ ] API Gateway stage cache configuration documented

## Notes

- The distinction between "latest ledger" (short TTL) and "historical ledger" (long TTL) requires the backend to know the current highest ledger.
- API Gateway cache configuration is an infrastructure concern but the NestJS headers drive the behavior.
- API Gateway infrastructure is defined in CDK task 0033.
