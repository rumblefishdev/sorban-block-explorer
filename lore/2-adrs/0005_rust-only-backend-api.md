---
id: '0005'
title: 'Rust-only backend: API and Ledger Processor both in Rust'
status: accepted
deciders: [stkrolikiewicz, fmazur]
related_tasks: ['0092', '0093', '0094']
related_adrs: ['0002', '0004']
tags: [architecture, rust, api, backend]
links:
  - https://github.com/tokio-rs/axum
  - https://github.com/actix/actix-web
history:
  - date: 2026-03-31
    status: proposed
    who: stkrolikiewicz
    note: 'ADR proposed after team meeting. Entire backend moves to Rust — not just Ledger Processor.'
  - date: 2026-03-31
    status: accepted
    who: stkrolikiewicz
    note: >
      Accepted. Research 0092 validated stack: axum 0.8 + utoipa 5.4 + sqlx 0.8 + lambda_http 1.1.
      PoC verified end-to-end. All backlog tasks being updated in 0093.
---

# ADR 0005: Rust-only backend — API and Ledger Processor both in Rust

**Related:**

- [ADR 0002: Use Rust for the Ledger Processor Lambda](0002_rust-ledger-processor-lambda.md)
- [ADR 0004: Rust-only XDR parsing](0004_rust-only-xdr-parsing.md)
- [Task 0092: Research Rust API stack](../1-tasks/active/0092_RESEARCH_rust-api-stack.md)

---

## Context

ADR 0002 moved the Ledger Processor to Rust while keeping the API in TypeScript (NestJS). ADR 0004 eliminated the TypeScript XDR decode path. After team discussion, the remaining NestJS API provides limited value — it's a CRUD layer over PostgreSQL with no XDR parsing, and the team has growing Rust expertise from the Ledger Processor work.

The NestJS API was bootstrapped (task 0023, completed) with modules scaffolded, but no business logic implemented yet. Cost of switching is low.

---

## Decision

Move the entire backend to Rust. Both the Ledger Processor Lambda and the REST API Lambda are Rust binaries. TypeScript is eliminated from the backend completely.

Frontend remains React/TypeScript.

---

## Rationale

### 1. Single backend language

One language for all backend code means one toolchain, one set of dependencies, one CI pipeline, shared types between API and Ledger Processor. No cross-language type duplication for DB schema, domain models, or error types.

### 2. Shared crate ecosystem

API and Ledger Processor can share Rust crates for:

- Database access (same ORM/query layer)
- Domain types (shared structs)
- Error handling (shared error types)
- Configuration (shared env resolution)

With NestJS API + Rust Processor, every shared concept required parallel implementations in two languages.

### 3. Performance consistency

Rust API Lambda has faster cold starts (~100-300ms vs ~500-1500ms Node.js) and lower memory. No provisioned concurrency needed. Lower Lambda cost.

### 4. Team direction

Team is investing in Rust expertise. Maintaining a NestJS codebase alongside Rust splits focus. Consolidating on Rust accelerates proficiency.

### 5. Low switching cost

NestJS API has scaffolded modules but zero business logic. No production traffic. No data migration. Switching now costs research + re-scaffolding, not rewriting.

---

## Alternatives Considered

### Alternative 1: Keep NestJS API (original design)

**Pros:**

- Team has TypeScript experience
- NestJS ecosystem (decorators, DI, Swagger, @nestjsx/crud)
- Already bootstrapped (task 0023)

**Cons:**

- Two languages in backend — type duplication, dual CI
- Node.js Lambda cold starts (500-1500ms)
- `@stellar/stellar-sdk` removed from API (ADR 0004) — NestJS has no Stellar-specific advantage
- Maintaining both Rust and TypeScript splits team focus

**Decision:** REJECTED — single-language backend is simpler and cheaper long-term.

### Alternative 2: Go for API

**Pros:**

- Stellar ecosystem is Go-first (Horizon, Galexie)
- Good Lambda performance

**Cons:**

- Three languages in project (Go API + Rust Processor + TypeScript frontend)
- No shared types between API and Processor
- Team doesn't have Go experience

**Decision:** REJECTED — adding a third language is worse than two.

---

## Consequences

### Positive

- Single backend language (Rust)
- Shared types and crates between API and Ledger Processor
- Faster Lambda cold starts, lower cost
- No `@nestjs/*`, `drizzle-orm`, `pg` runtime dependencies in backend
- Simpler CI (one Rust build pipeline)

### Negative

- NestJS bootstrapped work (task 0023) is discarded
- `libs/database` (Drizzle ORM) may become obsolete depending on Rust ORM research
- Team needs to learn Rust web framework (axum or actix-web)
- NestJS backend tasks in backlog (0043-0055) need cancellation or rewrite
- Loss of NestJS ecosystem (Swagger decorators, DI, middleware)

### Mitigations

- Research task (0092) evaluates Rust frameworks and ORM before committing
- Frontend stays React/TypeScript — no frontend disruption
- NestJS work was scaffolding only — no business logic lost
- Cleanup task (0093) handles backlog cancellation systematically

---

## Open Questions

1. **Rust REST framework:** axum vs actix-web — resolved by research task 0092
2. **Rust ORM/query layer:** sqlx vs diesel vs sea-orm — resolved by research task 0092
3. **Drizzle ORM fate:** Keep for migrations tooling, or replace with Rust ORM migrations? Depends on 0092 findings.
4. **Shared workspace structure:** Cargo workspace layout (apps/api-rs, apps/indexer-rs, libs/shared-rs?) — resolved by research task 0092

---

## References

- [ADR 0002: Rust Ledger Processor](0002_rust-ledger-processor-lambda.md)
- [ADR 0004: Rust-only XDR parsing](0004_rust-only-xdr-parsing.md)
- [axum](https://github.com/tokio-rs/axum)
- [actix-web](https://github.com/actix/actix-web)
