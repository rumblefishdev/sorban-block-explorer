---
type: synthesis
status: mature
spawned_from: 0092
tags: [final, stack, decision]
---

# Final Synthesis: Rust API Stack Recommendation

## Recommended Stack

| Layer          | Choice                    | Version       | Verified       |
| -------------- | ------------------------- | ------------- | -------------- |
| Web framework  | **axum**                  | 0.8.8         | cargo check ✅ |
| Lambda runtime | **lambda_http**           | 1.1.2         | cargo check ✅ |
| OpenAPI        | **utoipa** + utoipa-axum  | 5.4.0 / 0.2.0 | cargo check ✅ |
| Database       | **sqlx** (direct, no ORM) | 0.8.6         | cargo check ✅ |
| Middleware     | **tower-http**            | 0.6.8         | cargo check ✅ |
| Migrations     | **sqlx-cli** (plain SQL)  | —             | —              |
| Build tool     | **cargo-lambda**          | 1.9.1         | —              |
| CDK construct  | **cargo-lambda-cdk**      | —             | —              |

## Key Decisions

### 1. axum (not actix-web, not poem)

- Native Lambda integration via tower::Service → lambda_http::run()
- Tokio team maintenance (25,477 stars)
- Pure `#[tokio::main]` — zero runtime conflicts with stellar-xdr

### 2. utoipa (not aide)

- Better OpenAPI documentation: inline examples, `#[deprecated]`, per-router tags, clean schema naming
- 14x more downloads than aide (22.8M vs 1.6M)
- aide locked to stale schemars 0.9 (current stable is 1.2.1)
- CRUD base via `macro_rules!` — same total boilerplate as aide's generics

### 3. sqlx direct (no ORM)

- API is **read-only** — data written by Ledger Processor
- Compile-time SQL validation (`query!` / `query_as!`)
- Transparent partitioned tables, typed JSONB via `Json<T>`
- Lambda pool: `max_connections(1)`, `test_before_acquire(true)`
- RDS Proxy IAM auth: generate token via AWS SDK, pass as password

### 4. sqlx migrations (drop Drizzle Kit)

- Plain SQL files — full PostgreSQL DDL (PARTITION BY RANGE, custom indexes)
- Existing Drizzle migration trivially portable (1 file, strip breakpoint comments)
- `sqlx::migrate!()` embeddable in binary for cold-start migration check

### 5. Cargo workspace (4 crates)

- `api` (binary) — axum Lambda handler
- `indexer` (binary) — XDR ingestion Lambda
- `domain` (library) — shared types, errors, config (zero async deps)
- `db` (library) — sqlx pool, queries, migrations

Root `Cargo.toml` at monorepo root, crates under `rust/crates/`. Nx wraps cargo via `nx:run-commands`.

### 6. CI/CD

- Parallel Rust + Node.js GitHub Actions jobs
- `dtolnay/rust-toolchain@stable` + `Swatinem/rust-cache@v2`
- `cargo lambda build --release --arm64` (Zig cross-compile, no Docker)
- `SQLX_OFFLINE=true` + committed `.sqlx/`
- Cold: 7-10 min, cached: 2-3 min

## Cold Start Performance

lambda-perf (2026-03-30, provided.al2023, ARM64, ZIP):

- **Avg init: ~14ms** (range 12-19ms) — benchmark uses minimal hello-world Lambda
- **Memory: 15 MB** — minimal binary; our stack (axum+sqlx+utoipa) will use more
- Our binary is ~5.6 MB stripped — expect init ~20-40ms (higher than minimal due to larger binary + DB pool init)
- Still well below provisioned concurrency threshold (~200-500ms)
- No provisioned concurrency needed

## Acceptance Criteria Status

- [x] Framework recommendation — axum 0.8
- [x] ORM/query layer recommendation — sqlx 0.8.6 direct (ORM not needed)
- [x] Migration strategy — sqlx migrations, drop Drizzle Kit
- [x] Lambda deployment pattern — cargo-lambda build, handler via lambda_http::run()
- [x] Shared crate workspace layout — 4 crates (api, indexer, domain, db)
- [x] Cold start benchmarks — avg ~14ms ARM64 (verified from lambda-perf source data)
- [x] Proof of concept — `tools/scripts/api-stack-test/` (cargo check ✅)

## Proof of Concept

Location: `tools/scripts/api-stack-test/`

```
src/
├── main.rs        — Lambda handler, pool config, Swagger UI, router assembly
├── ledgers.rs     — GET /ledgers (list + cursor pagination), GET /ledgers/{sequence}
├── pagination.rs  — PaginationParams (IntoParams), PaginatedLedgers (ToSchema)
├── cursor.rs      — Opaque base64 cursor encode/decode
└── error.rs       — AppError enum with IntoResponse (thiserror)
```

Demonstrates:

- axum Router → lambda_http::run() (Lambda handler)
- sqlx query_as with FromRow + chrono DateTime (DB query)
- utoipa #[utoipa::path] + ToSchema + IntoParams (OpenAPI)
- utoipa-swagger-ui (Swagger UI at /swagger-ui)
- Cursor-based pagination (opaque base64, keyset pagination)
- Lambda pool config (max_connections=1, test_before_acquire)
- tower-http CORS + tracing middleware
- thiserror-based error handling with IntoResponse

All 7 acceptance criteria met.
