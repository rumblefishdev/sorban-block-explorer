---
id: '0092'
title: 'Research: Rust API stack — framework, ORM, Lambda deployment'
type: RESEARCH
status: active
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
---

# Research: Rust API stack — framework, ORM, Lambda deployment

## Summary

Evaluate and recommend the Rust technology stack for the block explorer REST API Lambda. Covers three areas: web framework, database access, and Lambda deployment patterns. CTO suggested axum and actix-web as starting points for framework research. Diesel is excluded from ORM evaluation (team decision).

## Status: Active

**Current state:** Research starting. Blocks all Rust API implementation tasks.

## Context

Team decided to move entire backend to Rust (ADR 0005). Need to choose:

1. **Web framework** — find the best Rust web framework for a Lambda-deployed REST API. Starting points from CTO: axum and actix-web. Research should also consider other viable options (e.g., warp, rocket, poem, lambda_http directly) and recommend the best fit.

2. **Database access** — query layer for PostgreSQL via RDS Proxy. Known options:

   - **sqlx** — compile-time checked SQL queries, no ORM. Raw SQL with type safety.
   - **sea-orm** — async ORM built on sqlx. ActiveRecord pattern. Migrations built-in.
   - Research should evaluate these and any other viable async options. **Diesel is excluded** (team decision — sync-first, doesn't fit async Lambda model).
   - Key factors: partitioned tables support, JSONB typing, migration system (replaces Drizzle?), async support, Lambda connection lifecycle.

3. **Lambda deployment** — cargo-lambda-cdk integration, binary size, cold start optimization, connection pooling with RDS Proxy.

## Research Questions

### Framework

- Q1: What are the viable Rust web frameworks for Lambda REST API? (axum, actix-web, warp, rocket, poem, raw lambda_http, others?)
- Q2: Cold start comparison on Lambda (provided.al2023, ARM64)?
- Q3: Middleware/extensibility ecosystem (auth, logging, CORS, rate limiting, tower compat)?
- Q4: cargo-lambda integration — how does each work as a Lambda handler?
- Q5: OpenAPI/Swagger generation support (utoipa, aide, or other)?
- Q6: Request validation and error handling patterns?
- Q7: Community size, maintenance activity, production adoption?

### ORM / Database

- Q8: What are the viable async Rust PostgreSQL query/ORM options? (sqlx, sea-orm, others? Not diesel.)
- Q9: Which handles partitioned tables (PARTITION BY RANGE) best?
- Q10: JSONB column typing — which gives compile-time safety on JSONB fields?
- Q11: Migration system — can it replace Drizzle Kit? Custom SQL for partitions?
- Q12: Connection lifecycle for Lambda — pool management with max 1 connection?
- Q13: BigInt/i64 handling — native i64 or needs conversion?
- Q14: Can existing Drizzle migrations be reused, or does Rust ORM need its own?

### Lambda Deployment

- Q15: Binary size comparison across frameworks?
- Q16: Shared crate structure — how to share types/db between API and Ledger Processor?

## Acceptance Criteria

- [ ] Framework recommendation with rationale — best option found, not limited to axum/actix-web
- [ ] ORM/query layer recommendation with rationale (diesel excluded)
- [ ] Migration strategy decided (keep Drizzle Kit, switch to Rust migrations, or plain SQL)
- [ ] Lambda deployment pattern documented (handler structure, connection lifecycle)
- [ ] Shared crate workspace layout proposed
- [ ] Cold start benchmarks or estimates for recommended stack
- [ ] Proof of concept: minimal Lambda endpoint with DB query using recommended stack

## Notes

- axum and actix-web are starting points, not constraints. Research should find the best option.
- Diesel is excluded from evaluation (team decision).
- stellar-indexer reference repo uses sqlx — check if that's a good baseline.
- Drizzle ORM fate depends on this research: if Rust ORM has good migrations, Drizzle can be removed.
- Frontend stays React/TypeScript — this research is backend only.
