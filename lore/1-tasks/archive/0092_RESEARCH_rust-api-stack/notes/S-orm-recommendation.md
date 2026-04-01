---
type: synthesis
status: mature
spawned_from: 0092
tags: [orm, sqlx, decision, migrations]
---

# Synthesis: ORM / Database Recommendation

**Decision: ORM not needed. sqlx 0.8.6 direct. sqlx migrations (drop Drizzle Kit).**

## Verified Stack (cumulative)

| Crate       | Version | Purpose                    | Verified                                                   |
| ----------- | ------- | -------------------------- | ---------------------------------------------------------- |
| axum        | 0.8.8   | Web framework              | cargo check ✅                                             |
| lambda_http | 1.1.2   | Lambda runtime             | cargo check ✅                                             |
| sqlx        | 0.8.6   | DB access + migrations     | cargo check ✅ (features: postgres, json, chrono, migrate) |
| utoipa      | 5.4.0   | OpenAPI generation         | cargo check ✅                                             |
| utoipa-axum | 0.2.0   | Route-level OpenAPI        | cargo check ✅                                             |
| tower-http  | 0.6.8   | Middleware (CORS, tracing) | cargo check ✅                                             |

All compile together (edition 2024, rustc 1.94.0).

## Why no ORM

This API is **read-only** — data is written by the Ledger Processor (separate binary). Queries are simple SELECTs with WHERE, ORDER BY, LIMIT, cursor pagination. No complex JOINs, no relationship loading, no INSERT/UPDATE/DELETE from the API. An ORM adds entities, ActiveModel, Column enums, Relation traits — all overhead for reads-only.

## Why sqlx direct

1. **Compile-time SQL validation** via `query!` / `query_as!` — catches typos, wrong column names, type mismatches at build time. Stronger than any ORM's runtime checks.
2. **Full PostgreSQL feature access** — partitioned tables, JSONB, custom indexes, triggers. No abstraction leaks.
3. **Zero overhead** — sqlx IS the postgres driver. `query_as!` + `#[derive(FromRow)]` is all we need.
4. **Lambda-optimized** — `PgPoolOptions::max_connections(1)` + `test_before_acquire(true)`. Proven freeze/thaw handling.
5. **RDS Proxy IAM auth** — generate token via AWS SDK, pass as password. SSL works out of the box with rustls + webpki-roots.
6. **stellar-indexer precedent** — our reference repo already uses sqlx.

## Other options evaluated and rejected

- **sea-orm** — unnecessary abstraction, loses compile-time SQL checking, `ActiveValue` verbose
- **tokio-postgres** — sqlx wraps it, adds compile-time safety + pool + migrations
- **cornucopia** — stale (last release Nov 2022), sqlx `query!` covers same ground
- **sea-query standalone** — optional for dynamic query building, not essential for 10 resources. Can add later.

## Migration strategy

Drop Drizzle Kit → sqlx migrations (plain SQL files). Existing 1-file Drizzle migration trivially portable. Future partition DDL is first-class in plain SQL.

## Verified patterns (cargo check)

- `sqlx::types::Json<OperationTree>` for typed JSONB ✅
- `DateTime<Utc>` for TIMESTAMPTZ (chrono feature) ✅
- `i64` for BIGINT (native mapping) ✅
- `PgPoolOptions::max_connections(1).test_before_acquire(true)` ✅
- utoipa `IntoParams` for pagination query params ✅
- Concrete `PaginatedLedgers` type (not generic with aliases — `#[aliases]` does not work in utoipa 5.4.0) ✅

## Next Step

Proceed to step 3 — Lambda deployment + Cargo workspace layout + CI implications (Q15-Q16).
