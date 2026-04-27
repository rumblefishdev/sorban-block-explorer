---
id: '0045'
title: 'Backend: Network module (GET /network/stats)'
type: FEATURE
status: active
related_adr: ['0005', '0008', '0021', '0027']
related_tasks: ['0042', '0043', '0046', '0092']
tags: [layer-backend, network, stats]
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
  - date: '2026-04-27'
    status: active
    who: karolkow
    note: 'Promoted — next backend module after 0043.'
---

# Backend: Network module (GET /network/stats)

## Summary

Implement the Network module providing the `GET /network/stats` endpoint. This is a small, fast, cacheable endpoint that serves top-level explorer summary data including ledger sequence, TPS, total accounts, total contracts, and ingestion freshness indicators.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per [ADR 0005](../../2-adrs/0005_rust-only-backend-api.md)). Envelope shape per [ADR 0008](../../2-adrs/0008_error-envelope-and-pagination-shape.md). Frontend display contract + TPS formula per [ADR 0021](../../2-adrs/0021_schema-endpoint-frontend-coverage-matrix.md) §E1. Endpoint realizability confirmed in [ADR 0027](../../2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md) §E1. Code in `crates/api/src/network/`. Reuses `crate::common::errors::*` from task 0043.

## Status: Active

**Current state:** Active — promoted 2026-04-27. Prereqs satisfied: api skeleton + `common/*` helpers landed via tasks 0042/0046/0043.

## Context

The network stats endpoint is the primary source of top-level explorer summary information. It is consumed by the explorer header/dashboard to show chain overview data. It must remain small, fast, and aggressively cached.

### API Specification

**Endpoint:** `GET /v1/network/stats`

**Method:** GET

**Path:** `/network/stats`

**Query Parameters:** None

**Response Shape:**

```json
{
  "tps": 42.5,
  "total_accounts": 1500000,
  "total_contracts": 25000,
  "highest_indexed_ledger": 12345678,
  "ingestion_lag_seconds": 3
}
```

**Response Fields:**

| Field                    | Type           | Description                                                       |
| ------------------------ | -------------- | ----------------------------------------------------------------- |
| `tps`                    | number         | Current transactions per second (60s rolling window per ADR 0021) |
| `total_accounts`         | number         | Total indexed account count                                       |
| `total_contracts`        | number         | Total indexed Soroban contract count                              |
| `highest_indexed_ledger` | number         | Highest ledger sequence present in the database                   |
| `ingestion_lag_seconds`  | number or null | Estimated seconds behind the network tip; null if unknown         |

### Freshness Indicator

- `highest_indexed_ledger` vs network tip communicates data freshness
- When ingestion is behind, the endpoint degrades gracefully: serves indexed data with accurate freshness indicator
- No error thrown solely because data is stale

### Caching

| Layer            | TTL    | Notes                                                    |
| ---------------- | ------ | -------------------------------------------------------- |
| API Gateway      | 5-15s  | Short TTL for near-real-time summary                     |
| Lambda in-memory | 30-60s | Module-level variable persisting across warm invocations |

### Error Handling

- 500 if database is unreachable (standard error envelope)
- No 400/404 scenarios for this endpoint (no params, resource always exists)

Per ADR 0008 — flat envelope, no outer `error` wrapper:

```json
{
  "code": "db_error",
  "message": "Unable to retrieve network statistics.",
  "details": null
}
```

Use `crate::common::errors::internal_error(errors::DB_ERROR, "Unable to retrieve network statistics.")` from task 0043.

### Data Sources

- `highest_indexed_ledger`: `SELECT max(sequence) FROM ledgers` (also used as `closed_at` source via `SELECT sequence, closed_at FROM ledgers ORDER BY sequence DESC LIMIT 1`)
- `tps`: per ADR 0021 — `SELECT count(*)::float / 60 FROM transactions WHERE created_at > now() - interval '1 minute'` (60s rolling window, source = `transactions`, NOT `ledgers.transaction_count`)
- `total_accounts`: `SELECT count(*) FROM accounts`
- `total_contracts`: `SELECT count(*) FROM soroban_contracts`
- `ingestion_lag_seconds`: `EXTRACT(EPOCH FROM now() - max(closed_at))::int FROM ledgers`; `null` when no ledgers indexed

## Implementation Plan

### Step 1: Network Route + handler setup

Create `crates/api/src/network/` with axum route module with handlers and query module. Register routes in the top-level `Router` in `crates/api/src/main.rs`.

### Step 2: Stats Query Implementation

Implement the database queries in the service layer:

- Latest ledger sequence from `ledgers` table
- TPS calculation from recent ledger transaction counts
- Total accounts count from `accounts` table
- Total contracts count from `soroban_contracts` table
- Ingestion lag from latest ledger `closed_at` vs current time

### Step 3: In-Memory Caching

Implement Lambda in-memory cache (module-level variable) with 30-60s TTL for the stats response. Cache is lost on cold start, which is acceptable.

### Step 4: Response Serialization

Map query results to the documented response shape. Ensure `ingestion_lag_seconds` is null when calculation is not possible (e.g., no ledgers indexed yet).

## Acceptance Criteria

- [ ] `GET /v1/network/stats` returns documented response shape
- [ ] `tps` computed per ADR 0021 60s-rolling formula on `transactions` table
- [ ] `total_accounts` and `total_contracts` are accurate counts
- [ ] `highest_indexed_ledger` reflects `max(sequence)` from `ledgers`
- [ ] `ingestion_lag_seconds` computed from latest ledger `closed_at`; null when no ledgers indexed
- [ ] In-memory cache with 30-60s TTL reduces DB round-trips
- [ ] Graceful degradation when ingestion is behind (no errors, accurate freshness)
- [ ] Response is small and fast (suitable for 5-15s API Gateway cache)
- [ ] Standard error envelope on failure

## Notes

- This is one of the simplest API endpoints but one of the most frequently called.
- The in-memory cache is critical for reducing database load from repeated dashboard refreshes.
- TPS calculation methodology should be documented in code comments for maintainability.
