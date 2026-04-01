---
type: research
status: mature
spawned_from: 0092
tags: [orm, sqlx, sea-orm, database, lambda, migrations]
---

# ORM / Database Research (Q8-Q14)

## Key Finding: ORM Not Needed

This is a **read-only API** — data is written by the Ledger Processor (separate Rust binary), not the API. Queries are simple SELECTs with WHERE, ORDER BY, LIMIT, cursor pagination. No complex JOINs, no graph traversal, no eager/lazy loading. For this use case, an ORM adds abstraction without value.

## Candidates Evaluated

| Library                    | Version                           | Type                                    | Verdict                                             |
| -------------------------- | --------------------------------- | --------------------------------------- | --------------------------------------------------- |
| **sqlx**                   | 0.8.6                             | Async SQL toolkit, compile-time checked | **RECOMMENDED**                                     |
| **sea-orm**                | 1.1.19 (stable), 2.0.0-rc.37 (RC) | ORM built on sqlx                       | Rejected — unnecessary abstraction                  |
| **tokio-postgres**         | 0.7.x                             | Raw async PostgreSQL driver             | Rejected — sqlx wraps it, adds safety               |
| **cornucopia**             | 0.9.0                             | SQL-file codegen                        | Rejected — stale (last release Nov 2022)            |
| **sea-query** (standalone) | 1.0.0-rc.32                       | Type-safe query builder                 | Optional — useful if dynamic queries get repetitive |
| diesel                     | —                                 | Sync ORM                                | Excluded (team decision)                            |

## sqlx 0.8.6 — Why Recommended

### Compile-time SQL validation (Q8)

`query!` / `query_as!` macros validate SQL against actual DB schema at compile time:

```rust
let ledger = sqlx::query_as!(Ledger,
    "SELECT sequence, hash, closed_at FROM ledgers WHERE sequence = $1",
    sequence
).fetch_one(&pool).await?;
```

CI: `cargo sqlx prepare` generates `.sqlx/` offline metadata. `SQLX_OFFLINE=true` in CI. `cargo sqlx prepare --check` validates.

Three query tiers: `query!` (compile-time), `query_as` (runtime typed), `QueryBuilder` (dynamic).

### Partitioned tables (Q9)

Transparent. PostgreSQL handles partition routing server-side. sqlx sends normal INSERT/SELECT to parent table. `query!` validates against parent table schema. No gotchas.

### JSONB typing (Q10)

`sqlx::types::Json<T>` wrapper (requires `json` feature):

```rust
#[derive(sqlx::FromRow)]
struct OperationRow {
    id: i64,
    operation_tree: Json<OperationTree>,
}
```

Compile-time type override: `operation_tree as "operation_tree: Json<OperationTree>"` in `query_as!`. Runtime deserialization safety via serde. No compile-time JSONB field validation (inherent JSONB limitation).

### Migration system (Q11)

Plain SQL files via `sqlx-cli`:

```bash
cargo sqlx migrate add create_operations
# → migrations/YYYYMMDDHHMMSS_create_operations.sql
```

Write any valid PostgreSQL DDL — PARTITION BY RANGE, custom indexes, triggers. No abstraction.

Embed in binary via `sqlx::migrate!().run(&pool).await` — migrations compiled into binary at build time.

**Replaces Drizzle Kit.** Drizzle's value (schema diff → SQL generation) is irrelevant without TypeScript schema source.

### Lambda connection lifecycle (Q12)

```rust
let pool = PgPoolOptions::new()
    .max_connections(1)          // Lambda: 1 request at a time
    .min_connections(0)          // Lazy connect on first query
    .acquire_timeout(Duration::from_secs(5))
    .idle_timeout(Some(Duration::from_secs(600)))
    .test_before_acquire(true)   // Ping before reuse — handles freeze/thaw
    .connect(&database_url).await?;
```

- Pool survives Lambda freeze/thaw
- `test_before_acquire(true)` detects dead connections after freeze (~1ms ping)
- RDS Proxy handles multiplexing across Lambda instances
- Cold start DB overhead: one TCP+TLS handshake to RDS Proxy (~10-50ms same VPC)

### BigInt/i64 (Q13)

Native zero-cost mapping: PostgreSQL `BIGINT`/`INT8`/`BIGSERIAL` ↔ Rust `i64`. No conversion needed.

### Drizzle migration reuse (Q14)

Existing `0000_create_ledgers_transactions.sql` is plain SQL. Strip `--> statement-breakpoint` comments, copy to `migrations/` with sqlx timestamp naming. 5-minute task.

### Verified feature flags

```toml
sqlx = { version = "0.8", features = [
    "runtime-tokio-rustls",
    "postgres",
    "json",      # Json<T> for JSONB
    "chrono",    # DateTime<Utc> for TIMESTAMPTZ
    "migrate",   # migrate!() macro
] }
```

Cargo check passed with full stack: axum 0.8.8 + lambda_http 1.1.2 + sqlx 0.8.6 (json+chrono+migrate) + utoipa 5.4.0 + tower-http 0.6.8. Edition 2024, rustc 1.94.0.

## sea-orm — Why Rejected

1. **Adds abstraction without value.** For 10-table CRUD API, sea-orm's entity system (Model, ActiveModel, Column enum, Relation) is proportionally heavy.
2. **Loses compile-time SQL validation.** sea-orm query builder has no equivalent to sqlx's `query!` macro.
3. **`ActiveValue::Set()`/`ActiveValue::NotSet` verbose.** Widely criticized ergonomics.
4. **Partitioned tables** — transparent (same as sqlx, no advantage).
5. **JSONB** — parity with sqlx (both use serde).
6. **Migrations** — sea-orm-migration uses Rust code, but you write raw SQL for partitions anyway.
7. **Code generation** — generates entities from DB, but "generate then customize" is awkward for 10 tables.
8. **2.0 still RC** (rc.37, Mar 2026). 1.x (1.1.19) is stable but no compelling reason to use it over sqlx.

**Where sea-orm makes sense:** Large schemas (50+ tables), teams wanting ActiveRecord patterns, multi-DB support. Not this project.

## Migration Strategy

**Decision: sqlx migrations. Drop Drizzle Kit.**

1. Create `migrations/` in Rust API crate
2. Port existing Drizzle SQL file (strip `--> statement-breakpoint`)
3. Future tables: `cargo sqlx migrate add <name>` → write raw SQL
4. CI: `cargo sqlx migrate run` as pipeline step before Lambda deploy
5. Local dev: embedded `sqlx::migrate!()` or docker-compose init
6. Archive `libs/database/` when frontend no longer needs it

## Other Options Evaluated (Q8 completeness)

### tokio-postgres (raw driver)

sqlx is built on top of it. sqlx adds: compile-time SQL validation, auto row-to-struct mapping, built-in pool, migrations. Zero reason to go lower-level for read-only CRUD.

### cornucopia (SQL-file codegen)

Generates Rust code from .sql files, compile-time checked. **Stale**: last release Nov 2022, 3+ years old, 50 open issues. Risk too high for production. sqlx `query!` covers same ground.

### sea-query standalone (query builder, no ORM)

v1.0.0-rc.32 (Mar 2026), very active (27.6M downloads). Works with sqlx via `sea-query-binder`. Good for dynamic query composition (optional filters, pagination). **Not essential** for 10 resources with simple queries — can add later if pattern warrants it.

## RDS Proxy Connection (Lambda)

### IAM Authentication

sqlx has **no built-in IAM auth**. Generate token via AWS SDK, pass as password:

```rust
let token = rds_client.generate_db_auth_token()
    .hostname("proxy.xxxx.us-east-1.rds.amazonaws.com")
    .port(5432).username("lambda_role").send().await?;
let url = format!("postgres://lambda_role:{}@{}:5432/db?sslmode=require",
    urlencoding::encode(&token), proxy_host);
```

Token lasts 15 min. Set `max_lifetime(Duration::from_secs(600))` on pool to force reconnection before expiry.

### SSL/TLS

`runtime-tokio-rustls` includes webpki-roots with Amazon Trust Services root CAs. RDS Proxy certs chain to these — **works out of the box** with `sslmode=require`.

### Known Issues

- **Prepared statement pinning**: RDS Proxy supports prep stmts without pinning since 2023. If issues arise: `.statement_cache_capacity(0)`.
- **SET commands, advisory locks, temp tables** cause pinning — avoid in Lambda handlers.

## utoipa 5.x Generic Types Correction

`#[aliases]` was **removed in utoipa 5.x** (existed in 4.x only). The utoipa 5.x approach for generic types:

1. `#[derive(ToSchema)]` on generic struct works: `PaginatedResponse<T: ToSchema>`
2. Register concrete types in `#[openapi]` via turbofish: `schemas(PaginatedResponse::<Ledger>)`
3. Use Rust type aliases for `body = inline(...)` in handler annotations (avoids `<>` parse issue): `type PaginatedLedgers = PaginatedResponse<Ledger>`

**Verified in PoC (cargo check ✅).** Generic `PaginatedResponse<T>` works — no need for concrete structs per resource. The `crud_routes!` macro generates type aliases (1 line per resource), not full structs.

## Sources

- sqlx 0.8.6 API docs: https://docs.rs/sqlx/0.8.6/sqlx/
- sqlx PostgreSQL types: https://docs.rs/sqlx/0.8.6/sqlx/postgres/types/
- sqlx-cli: https://github.com/launchbadge/sqlx/blob/main/sqlx-cli/README.md
- sea-orm 1.1.19 API docs (stable): https://docs.rs/sea-orm/1.1.19/sea_orm/
- sea-orm website docs (targets 2.0.x RC, not stable): https://www.sea-ql.org/SeaORM/docs/introduction/sea-orm/
- sea-orm on crates.io: https://crates.io/crates/sea-orm
- Cargo check verification: api-stack-test project (local)
