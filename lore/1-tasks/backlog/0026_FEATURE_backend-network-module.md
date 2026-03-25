---
id: '0026'
title: 'Backend: Network module (GET /network/stats)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023']
tags: [layer-backend, network, stats]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: Network module (GET /network/stats)

## Summary

Implement the Network module providing the `GET /network/stats` endpoint. This is a small, fast, cacheable endpoint that serves top-level explorer summary data including ledger sequence, TPS, total accounts, total contracts, and ingestion freshness indicators.

## Status: Backlog

**Current state:** Not started. Depends on task 0023 (NestJS API bootstrap).

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
  "ledger_sequence": 12345678,
  "tps": 42.5,
  "total_accounts": 1500000,
  "total_contracts": 25000,
  "highest_indexed_ledger": 12345678,
  "ingestion_lag_seconds": 3
}
```

**Response Fields:**

| Field                    | Type           | Description                                                    |
| ------------------------ | -------------- | -------------------------------------------------------------- |
| `ledger_sequence`        | number         | Latest known ledger sequence from the database                 |
| `tps`                    | number         | Current transactions per second (computed from recent ledgers) |
| `total_accounts`         | number         | Total indexed account count                                    |
| `total_contracts`        | number         | Total indexed Soroban contract count                           |
| `highest_indexed_ledger` | number         | Highest ledger sequence present in the database                |
| `ingestion_lag_seconds`  | number or null | Estimated seconds behind the network tip; null if unknown      |

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

```json
{
  "error": {
    "code": "INTERNAL_ERROR",
    "message": "Unable to retrieve network statistics."
  }
}
```

### Data Sources

- `ledger_sequence` / `highest_indexed_ledger`: MAX(sequence) from `ledgers` table
- `tps`: computed from transaction_count of recent ledgers divided by time window
- `total_accounts`: COUNT from `accounts` table
- `total_contracts`: COUNT from `soroban_contracts` table
- `ingestion_lag_seconds`: difference between now and `closed_at` of the most recent ledger

## Implementation Plan

### Step 1: Network Module Scaffolding

Create `apps/api/src/network/` with NestJS module, controller, and service. Register in AppModule.

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
- [ ] `ledger_sequence` reflects latest indexed ledger
- [ ] `tps` computed from recent ledger data
- [ ] `total_accounts` and `total_contracts` are accurate counts
- [ ] `highest_indexed_ledger` matches `ledger_sequence`
- [ ] `ingestion_lag_seconds` computed from latest ledger close time; null if unavailable
- [ ] In-memory cache with 30-60s TTL reduces DB round-trips
- [ ] Graceful degradation when ingestion is behind (no errors, accurate freshness)
- [ ] Response is small and fast (suitable for 5-15s API Gateway cache)
- [ ] Standard error envelope on failure

## Notes

- This is one of the simplest API endpoints but one of the most frequently called.
- The in-memory cache is critical for reducing database load from repeated dashboard refreshes.
- TPS calculation methodology should be documented in code comments for maintainability.
