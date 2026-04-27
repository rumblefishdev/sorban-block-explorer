---
id: '0045'
title: 'Backend: Network module (GET /network/stats)'
type: FEATURE
status: completed
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
  - date: '2026-04-27'
    status: completed
    who: karolkow
    note: >
      Implemented `crates/api/src/network/` (5 files, ~390 LOC + ~120 LOC tests).
      Combined ledgers SELECT for highest_indexed_ledger + ingestion_lag_seconds.
      30s in-memory cache via OnceLock<Mutex>. Cache-Control header
      `public, max-age=10` matches infra/envs apiGatewayCacheTtlMutable.
      `crate::common::errors` cherry-picked from feat/0043 branch (slim mod.rs);
      0045 PR depends on 0043 merge order. 24/24 api-bin tests pass,
      cargo clippy --all-targets -D warnings clean.
---

# Backend: Network module (GET /network/stats)

## Summary

Implement the Network module providing the `GET /network/stats` endpoint. This is a small, fast, cacheable endpoint that serves top-level explorer summary data including ledger sequence, TPS, total accounts, total contracts, and ingestion freshness indicators.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per [ADR 0005](../../2-adrs/0005_rust-only-backend-api.md)). Envelope shape per [ADR 0008](../../2-adrs/0008_error-envelope-and-pagination-shape.md). Frontend display contract + TPS formula per [ADR 0021](../../2-adrs/0021_schema-endpoint-frontend-coverage-matrix.md) §E1. Endpoint realizability confirmed in [ADR 0027](../../2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md) §E1. Code in `crates/api/src/network/`. Reuses `crate::common::errors::*` from task 0043.

## Status: Completed

**Current state:** Implemented 2026-04-27. Endpoint live at `/v1/network/stats`, 4 unit + 1 DB-gated integration test pass. Branch `feat/0045_backend-network-module` depends on `feat/0043_backend-pagination-query-parsing` for `crate::common::errors::*` (cherry-picked); 0045 PR merges after 0043.

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

- [x] `GET /v1/network/stats` returns documented response shape — `network::handlers::get_network_stats`
- [x] `tps` computed per ADR 0021 60s-rolling formula on `transactions` table — `network::queries::fetch_stats`
- [x] `total_accounts` and `total_contracts` are accurate counts — `SELECT count(*)` on each table
- [x] `highest_indexed_ledger` reflects `max(sequence)` from `ledgers` — `coalesce(max(sequence), 0)::BIGINT`
- [x] `ingestion_lag_seconds` computed from latest ledger `closed_at`; null when no ledgers indexed — CASE WHEN max(closed_at) IS NULL
- [x] In-memory cache with 30-60s TTL reduces DB round-trips — `network::cache` (`OnceLock<Mutex<Option<(Instant, NetworkStats)>>>`, TTL = 30s)
- [x] Graceful degradation when ingestion is behind (no errors, accurate freshness) — endpoint always returns 200 with the data we have; lag value communicates freshness
- [x] Response is small and fast (suitable for 5-15s API Gateway cache) — single static SQL round (4 queries, sequential), ~80-byte JSON
- [x] Standard error envelope on failure — `errors::internal_error(errors::DB_ERROR, ...)` (cherry-picked from task 0043)
- [x] `Cache-Control: public, max-age=10` header set on every 200 response — matches `apiGatewayCacheTtlMutable` in `infra/envs/{staging,production}.json` so the gateway cache cluster (when enabled) honours the same freshness budget as the in-memory cache

## Design Decisions

### From Plan

1. **TPS formula per ADR 0021** — `count(*)::float8 / 60.0 FROM transactions WHERE created_at > now() - interval '1 minute'`. Source = `transactions` partitioned fact table, predicate hits the current partition only. Locked in pre-implementation.
2. **Lambda in-memory cache 30-60s** — per `docs/architecture/backend/backend-overview.md` §8.x. Settled on 30s (lower bound) to keep perceived staleness aligned with API Gateway 5-15s TTL.
3. **Drop `ledger_sequence` field, keep `highest_indexed_ledger`** — pre-impl spec realign decision. Avoids redundant pair when no external network-tip fetch is wired.
4. **Drop `closed_at` from response** — frontend computes via `ingestion_lag_seconds`; redundant.
5. **Reuse `crate::common::errors::*` from task 0043** — single source of truth for error envelope shape per ADR 0008.

### Emerged

6. **Combined `ledgers` SELECT** — original plan had 5 separate queries (one per field). Single combined SELECT returns `highest_indexed_ledger` + `ingestion_lag_seconds` together, saving one round-trip on the cache-miss path. Net: 4 queries instead of 5.
7. **`OnceLock<Mutex<Option<(Instant, NetworkStats)>>>` for cache primitive** — spec said "module-level variable", picked std primitive over `arc_swap` to avoid a new dependency. Critical section is a single pointer write, contention irrelevant for the access pattern (1 miss / ~30s per warm Lambda).
8. **Empty-DB sentinel `coalesce(max(sequence), 0)::BIGINT`** — Stellar genesis is ledger 1, so `0` is an unambiguous "no data" signal. Documented in `dto.rs` so frontend can treat it as bootstrap state.
9. **`Cache-Control: public, max-age=10` response header** — not in original spec AC. Added to match `apiGatewayCacheTtlMutable: 10` in `infra/envs/{staging,production}.json` so the gateway cache cluster (when enabled) honours the same freshness budget. AC retroactively added for the audit trail.
10. **`ok_response()` helper centralising headers** — single source of truth so cache-hit and cache-miss paths cannot drift on the header set if more headers are added later.
11. **Cherry-pick subset of `common/*` from `feat/0043` branch** — only `errors.rs` + slim `mod.rs` pulled in. Full `common/*` set arrives when 0043 merges to `develop`. Slim `common/mod.rs` carries `#[allow(dead_code)]` on `pub mod errors;` as a temporary marker; allow disappears naturally once 0050+ adopt the unused codes (`INVALID_*`, `bad_request*`, `not_found`). 0045 PR depends on 0043 PR — merge order: 0043 then 0045.
12. **Cache-miss path treats mutex poison as miss** — `.lock().ok()?` returns None, handler refetches. No explicit poison recovery; in practice the lock cannot poison (no panic across the critical section).

## Issues Encountered

- **`feat/0045_backend-network-module` branched from `develop` before 0043 merged** — develop only carried the `chore(lore-0045): activate task` lore commit, not the `common/*` helpers. Resolved by cherry-picking `crates/api/src/common/errors.rs` from `feat/0043_backend-pagination-query-parsing` into 0045 branch as a pristine copy. Trade-off: clean rebase post-0043-merge vs temporary file divergence. Documented in `common/mod.rs` so the next reader knows this is a workaround, not a long-term layout choice.
- **Pre-push hook `cargo clippy --all-targets -- -D warnings`** flagged `INVALID_CURSOR/_LIMIT/_FILTER`, `bad_request*`, `not_found` as dead code on the cherry-picked `errors.rs`. Resolved with `#[allow(dead_code)]` on the `pub mod errors;` declaration in `common/mod.rs` instead of editing the file body — keeps `errors.rs` byte-identical with 0043 for clean rebase.
- **`count(*)` in PostgreSQL returns BIGINT, not NUMERIC** — confirmed on first compile (`query_scalar::<_, i64>` decoded fine). No `rust_decimal` dependency needed.

## Implementation Notes

- **Files added under `crates/api/src/network/`**: `mod.rs`, `dto.rs` (`NetworkStats` ToSchema), `queries.rs` (4 sqlx queries — ledgers combined SELECT + tps + accounts + contracts), `cache.rs` (`OnceLock<Mutex<Option<(Instant, NetworkStats)>>>` 30s TTL), `handlers.rs` (`get_network_stats` + `ok_response` helper centralising `Cache-Control` header + DB-gated integration test).
- **Combined ledgers query** — single SELECT returns both `highest_indexed_ledger` and `ingestion_lag_seconds`, saving one round-trip on the cache-miss path.
- **`crate::common::errors`** cherry-picked from task 0043 branch (only `errors.rs` + slim `mod.rs`); full `common/*` set arrives when 0043 merges to `develop`. The slim `common/mod.rs` carries `#[allow(dead_code)]` on `pub mod errors;` because only `DB_ERROR` + `internal_error` are consumed today; the allow disappears naturally as 0050+ endpoints adopt the other codes.
- **OpenAPI** — `NetworkStats` schema auto-registers via `routes!` macro; assertion test added to `main::tests::api_docs_json_contains_network_stats_path`.

## Notes

- One of the simplest API endpoints but one of the most frequently called.
- The in-memory cache is critical for reducing database load from repeated dashboard refreshes.
- TPS calculation methodology documented in `queries.rs` doc-comment.
