---
id: '0092'
title: 'Research: Rust API stack — framework, ORM, Lambda deployment'
type: RESEARCH
status: completed
related_adr: ['0005']
related_tasks: ['0002', '0023']
tags: [priority-high, effort-medium, layer-backend, rust]
milestone: 1
links:
  - https://github.com/tokio-rs/axum
  - https://github.com/actix/actix-web
  - https://github.com/launchbadge/sqlx
  - https://github.com/SeaQL/sea-orm
history:
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Task created after team decision to move entire backend to Rust (ADR 0005)'
  - date: 2026-03-31
    status: active
    who: stkrolikiewicz
    note: 'Activated for research'
  - date: 2026-03-31
    status: completed
    who: stkrolikiewicz
    note: >
      Research complete. 7/7 acceptance criteria met. PoC verified end-to-end
      (cargo lambda watch + curl + PostgreSQL).
      Stack: axum 0.8.8 + utoipa 5.4.0 + sqlx 0.8.6 + lambda_http 1.1.2.
      Deliverables: 4 R-notes, 3 S-syntheses, 4 verified sources, 1 PoC.
      Key decisions: axum (not actix-web/poem), utoipa (not aide), sqlx direct
      (no ORM), sqlx migrations (drop Drizzle Kit), 4-crate Cargo workspace.
---

# Research: Rust API stack — framework, ORM, Lambda deployment

## Summary

Evaluated and recommended Rust technology stack for block explorer REST API Lambda. Covers web framework, database access, Lambda deployment, Cargo workspace layout, and CI/CD.

## Status: Completed

**Result:** axum 0.8 + utoipa 5.4 + sqlx 0.8 + lambda_http 1.1. See S-final-stack-recommendation.md.

## Acceptance Criteria

- [x] Framework recommendation — axum 0.8 (8 frameworks evaluated, verified)
- [x] ORM/query layer recommendation — sqlx 0.8.6 direct, no ORM needed (read-only API)
- [x] Migration strategy — sqlx migrations, drop Drizzle Kit
- [x] Lambda deployment pattern — cargo-lambda build, lambda_http::run()
- [x] Shared crate workspace layout — 4 crates (api, indexer, domain, db)
- [x] Cold start benchmarks — avg ~14ms ARM64 (lambda-perf verified), est. ~20-40ms for our stack
- [x] Proof of concept — tools/scripts/api-stack-test/ (end-to-end verified)

## Implementation Notes

### Deliverables

```
notes/
├── R-framework-comparison.md        — 8 frameworks, all versions verified
├── R-openapi-crud-pagination.md     — aide vs utoipa: CRUD, pagination, docs
├── R-orm-database.md                — sqlx vs 5 alternatives + RDS Proxy
├── R-deployment-workspace-ci.md     — workspace layout, CI/CD, cargo-lambda
├── S-framework-recommendation.md    — axum + utoipa decision
├── S-orm-recommendation.md          — sqlx direct, no ORM
└── S-final-stack-recommendation.md  — complete stack summary

sources/
├── crates-io-verified.md            — version data from crates.io API
├── github-repos-verified.md         — stars/activity from GitHub API
├── github-reference-projects.md     — 5 reference projects
└── lambda-perf-rust-benchmarks.md   — cold start data with corrections

tools/scripts/api-stack-test/        — PoC (end-to-end verified)
├── src/main.rs                      — Lambda handler + pool + Swagger UI
├── src/ledgers.rs                   — CRUD handlers with utoipa annotations
├── src/pagination.rs                — Generic PaginatedResponse<T>
├── src/cursor.rs                    — Opaque base64 encode/decode
└── src/error.rs                     — AppError with IntoResponse
```

### Verification methods

- crates.io API for version numbers
- GitHub API (`gh api`) for stars, last push, releases
- lambda-perf source data for cold start benchmarks
- `cargo check` for crate compatibility
- `cargo lambda watch` + `curl` + PostgreSQL for end-to-end PoC

## Design Decisions

### From Plan

1. **axum as web framework** — native Lambda integration (tower::Service → lambda_http::run), tokio team maintenance, 25k+ stars. Actix-web eliminated (dead Lambda adapter). Poem rejected (bus factor 83% single maintainer).

2. **utoipa for OpenAPI** — better documentation quality (inline examples, #[deprecated], per-router tags) outweighs aide's CRUD ergonomics. aide locked to stale schemars 0.9.

3. **sqlx direct, no ORM** — read-only API, compile-time SQL validation, transparent partitioning. sea-orm rejected (unnecessary abstraction for 10-table CRUD).

4. **sqlx migrations** — plain SQL files replace Drizzle Kit. Existing migration trivially portable.

5. **4-crate Cargo workspace** — api (binary), indexer (binary), domain (library, zero async), db (library, sqlx).

### Emerged

6. **utoipa 5.x removed `#[aliases]`** — discovered during PoC. Workaround: Rust type aliases + turbofish syntax for generic schemas. `type PaginatedLedgers = PaginatedResponse<Ledger>` in `inline()`, `PaginatedResponse::<Ledger>` in `schemas()`.

7. **Cold start caveat** — lambda-perf benchmarks (~14ms) are for minimal hello-world Lambda. Our stack (axum+sqlx+utoipa, 5.6 MB binary) estimated ~20-40ms. Still no provisioned concurrency needed.

8. **aide evaluated deeply then rejected** — initially recommended aide for less CRUD boilerplate. After CRUD+pagination+documentation deep-dive, utoipa won 4-2 on documentation quality. aide's gaps (no #[deprecated], no per-router tags, weak param examples, ugly generic schema names) are harder to fix.

9. **schemars 0.9 is transitional** — aide 0.15 locked to schemars 0.9 while current stable is 1.2.1. Upgrade requires aide 0.16 (alpha). Additional reason to prefer utoipa (own schema system, no schemars dependency).

## Issues Encountered

- **Agent hallucinated lambda_http v0.14** — actual is v1.1.2. Caught by crates.io verification.
- **Agent hallucinated cold start P99 40-60ms** — that's container images, not ZIP. Actual ZIP max: 18.82ms. Caught by fetching lambda-perf source data.
- **Agent claimed binary size ~0.8 MB** — actual 2.6 MB gzipped (measured). Corrected.
- **`#[aliases]` compile error** — initially assumed working, turned out removed in utoipa 5.x. Required multiple iterations to find working pattern (type alias + turbofish).
- **sea-orm docs URL** — `https://www.sea-ql.org/SeaORM/docs/` returns 404 (bare path). Correct: `/docs/introduction/sea-orm/` (and that targets 2.0 RC, not stable 1.x).

## Future Work

- Scaffold actual Cargo workspace (separate implementation task)
- Implement CRUD `macro_rules!` pattern for 10 resources
- CDK integration with `cargo-lambda-cdk` RustFunction
- CI/CD GitHub Actions Rust job
- RDS Proxy IAM auth integration (verify AWS SDK for Rust syntax)
