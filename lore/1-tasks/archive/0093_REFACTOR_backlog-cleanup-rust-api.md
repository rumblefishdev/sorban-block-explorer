---
id: '0093'
title: 'Backlog cleanup: cancel NestJS tasks, update CDK and indexer tasks for Rust API'
type: REFACTOR
status: completed
related_adr: ['0005']
related_tasks: ['0092']
tags: [priority-high, effort-small, layer-meta]
milestone: 1
links: []
history:
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Task created. Depends on 0092 (research) for ORM/migration decisions before finalizing cleanup.'
  - date: 2026-03-31
    status: active
    who: stkrolikiewicz
    note: 'Activated — 0092 research complete, decisions made. Ready to clean up backlog.'
  - date: 2026-04-01
    status: completed
    who: stkrolikiewicz
    note: >
      37 files changed. 33 backlog tasks updated, 1 canceled (0091).
      ADR 0005 accepted. 6 docs/architecture files + README updated.
      Wiki snapshot created. Zero NestJS/Drizzle refs remaining in active content.
---

# Backlog cleanup: cancel NestJS tasks, update CDK and indexer tasks for Rust API

## Summary

Systematically updated the backlog after ADR 0005 (Rust-only backend). Canceled NestJS-specific tasks, updated CDK/indexer/backend tasks for Rust stack, updated docs.

## Acceptance Criteria

- [x] All NestJS-only tasks canceled or superseded with reason — 0091 canceled (obsolete)
- [x] Backend module tasks (0042-0055, 0057) updated for Rust (axum + utoipa + sqlx)
- [x] CDK tasks (0033, 0035) updated to reference Rust API Lambda
- [x] libs/database fate decided and documented (stays until 0094 migrates to crates/db/)
- [x] docs/architecture updated (6 files + README.md)
- [x] Wiki snapshot created (architecture-snapshot-rust-backend.md)
- [x] Lore index regenerated

## Implementation Notes

### Files changed (37 total)

**ADR:** 0005 (proposed → accepted)

**DB schema (4):** 0018, 0019, 0020 (Drizzle → sqlx plain SQL), 0021 (rewritten for sqlx migration framework)

**Backend modules (14):** 0042-0055, 0057 (NestJS → axum + utoipa + sqlx, apps/api/ → crates/api/)

**CDK + CI (3):** 0033 (Node.js Lambda → RustFunction), 0035 (NestJS Swagger → utoipa-swagger-ui), 0039 (added Rust CI job)

**Indexer/workers (4):** 0028-0030 (paths → crates/indexer/), 0056 (workers → Rust)

**Tests (1):** 0088 (NestJS patterns → cargo test + tokio::test)

**Canceled (1):** 0091 (NestJS local dev — obsolete, replaced by cargo lambda watch)

**Docs (7):** backend-overview, infrastructure-overview, technical-design-general-overview, xdr-parsing-overview, frontend-overview, README.md

**Wiki (1):** architecture-snapshot-rust-backend.md

## Design Decisions

### From Plan

1. **Rewrite, not cancel, framework-agnostic tasks** — tasks like pagination (0043), search (0053) have agnostic business logic. Rewrote implementation approach (NestJS → axum) while preserving scope.

2. **Cancel 0091 (NestJS local dev)** — fully NestJS-specific, no Rust equivalent needed. `cargo lambda watch` covers local dev (documented in research 0092).

3. **ADR 0005 accepted** — research 0092 validated the decision with PoC. Both deciders (stkrolikiewicz, fmazur) approved.

### Emerged

4. **Also updated CI task 0039** — original plan didn't mention CI/CD. Added Rust build job section (dtolnay/rust-toolchain, cargo-lambda, SQLX_OFFLINE).

5. **Also updated 0035 (CloudFront)** — had NestJS Swagger UI reference. Not in original plan but caught by full backlog scan.

6. **Also updated 0088 (tests)** — NestJS test patterns needed rewriting for Rust. Not in original plan steps.

7. **docs/architecture had 7 NestJS refs in technical-design-general-overview** — tables and timeline section referenced NestJS. All updated.

## Issues Encountered

- **Subagents hit permission issues** — 5 parallel agents + 1 batch agent all denied Edit/Write/Bash permissions. Had to complete manually. The batch agent (a220ec539938526bb) managed to update 33 files via 170 tool calls before hitting rate limit.

- **README.md, xdr-parsing-overview, frontend-overview missed by agent** — agent rate-limited before reaching these 3 files. Completed manually.

## Future Work

- Task 0094: Scaffold Cargo workspace (structural prerequisite for Rust implementation)
- Task 0095: Monorepo restructure (move web/, flatten infra/)
- Task 0096: OpenAPI → TypeScript codegen
