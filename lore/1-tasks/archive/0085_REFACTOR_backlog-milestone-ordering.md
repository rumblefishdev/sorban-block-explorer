---
id: '0085'
title: 'Reindex backlog tasks by deliverable milestone and clean up architecture'
type: REFACTOR
status: completed
related_adr: ['0003', '0004']
related_tasks: []
tags: [priority-high, effort-medium, layer-process]
milestone: 1
links:
  - docs/architecture/technical-design-general-overview.md
  - docs/architecture/xdr-parsing/xdr-parsing-overview.md
  - docs/architecture/backend/backend-overview.md
history:
  - date: 2026-03-30
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-30
    status: active
    who: fmazur
    note: 'Promoted to active'
  - date: 2026-03-30
    status: completed
    who: fmazur
    note: >
      Reindexed 67 backlog tasks by milestone (M1: 0016-0042, M2: 0043-0077+0086-0087, M3: 0088-0090).
      Created 5 new tasks (0041, 0042, 0088-0090). Removed 1 obsolete task (old 0035 XDR decode helpers).
      Removed 13 dead XDR files + @stellar/stellar-sdk deps. Updated all related_tasks across 90 tasks.
      2 ADRs created (0003 milestone ordering, 0004 Rust-only XDR). 2 architecture docs updated.
---

# Reindex backlog tasks by deliverable milestone and clean up architecture

## Summary

Backlog tasks were numbered by layer (DB → API → Frontend → Indexer → Infra), not by deliverable milestone. D1 tasks (indexing pipeline, CDK infra) had IDs 0060–0078 — at the end of the backlog — even though D1 must be completed first. This task reindexed all backlog tasks so IDs follow milestone order (M1 → M2 → M3), created missing tasks for gaps in D1/D3 coverage, eliminated the obsolete TS on-demand XDR decode path (per ADR 0004), and updated all cross-references.

## Status: Completed

**Current state:** Done.

## Context

The technical design (`docs/architecture/technical-design-general-overview.md` §7.4) defines three deliverables:

| Deliverable                             | Scope                                                                                    | Budget |
| --------------------------------------- | ---------------------------------------------------------------------------------------- | ------ |
| **D1** — Indexing Pipeline & Core Infra | Galexie ECS, Lambda Processor, XDR parsing, DB core tables, CDK infra, CI/CD, CloudWatch | 20%    |
| **D2** — Complete API + Frontend        | All REST endpoints, React SPA, event interpretation, caching                             | 30%    |
| **D3** — Mainnet Launch                 | Production deploy, tests, load tests, security audit, monitoring                         | 40%    |

Original task numbering put D1 infrastructure tasks after all D2 tasks. Working in ID order would complete D1 last instead of first.

## What Was Done

### 1. Reindexed all backlog tasks by milestone

All 67 backlog tasks were renumbered so IDs follow deliverable order:

- **M1 (0016–0042):** 27 tasks — DB schemas, NestJS scaffold, XDR parsing, Indexer, CDK infra, CI/CD, Galexie config, Swagger infra
- **M2 (0043–0077, 0086–0087):** 37 tasks — Backend API modules, Event Interpreter, OpenAPI docs, MUI theme, UI components, Frontend pages
- **M3 (0088–0090):** 3 tasks — Unit/integration tests, load testing, security audit

Gap at 0079–0085 is unavoidable (archive tasks 0079–0084, active task 0085).

Within each milestone, dependency order is respected:

- M1: DB schemas → XDR parsing → idempotent writes → handler → backfill → CDK → Galexie config → Swagger
- M2: Backend cross-cutting → feature modules → Event Interpreter → OpenAPI → MUI theme → UI components → frontend setup → pages
- M3: Tests → load test → security audit

### 2. Updated all cross-references

- All `related_tasks` in backlog tasks remapped to new IDs (verified: 0 mismatches, 0 dangling refs)
- All `related_tasks` in archive tasks (21 files) remapped to new IDs
- All `related_tasks` in active tasks remapped to new IDs
- `id` fields in frontmatter match filenames (verified: 0 mismatches)

### 3. Added `milestone` field to all tasks

- All backlog tasks: `milestone: 1`, `2`, or `3` in frontmatter
- All archive tasks: `milestone: 1`
- All active tasks: `milestone: 1`

### 4. Created missing tasks for gap coverage

- **0041** (Galexie configuration and testnet validation) — D1 effort breakdown item "Galexie configuration and testnet validation — 3 days" had no corresponding task. Task 0034 (CDK ECS Fargate) only covers infrastructure definition, not application configuration.
- **0042** (OpenAPI/Swagger infrastructure setup) — Split from old 0038 (now 0057). D1 scope includes "OpenAPI specification" but full endpoint documentation (0057) requires all API modules (M2). This task delivers the M1 infra part: swagger setup, document builder, spec export pipeline.
- **0088** (Unit and integration tests) — D3 scope item, 20 days in effort breakdown
- **0089** (Load testing) — D3 scope item, 4 days in effort breakdown
- **0090** (Security audit) — D3 scope item, 3 days in effort breakdown

### 5. Removed obsolete task and code (per ADR 0004)

**Removed task:**

- Old 0035 "Backend: API-time XDR decode helpers for advanced transaction view" — obsolete on green path. All XDR parsing is Rust-only at ingestion time.

**Removed code:**

- `libs/shared/src/xdr/` — 13 files (decoders, extractors, tests): scval-decoder, event-decoder, invocation-decoder, ledger-entry-extractors, contract-interface, transaction-utils + their tests + barrel index
- `libs/shared/src/index.ts` — removed XDR re-exports
- `libs/shared/dist/` — rebuilt clean (no XDR artifacts)

**Removed dependencies:**

- `@stellar/stellar-sdk` from root `package.json`
- `@stellar/stellar-base` + `@stellar/stellar-sdk` from `libs/shared/package.json`

**Kept:** `libs/shared/src/errors.ts` and `error-handlers.ts` — parse error types used by future Rust indexer integration, no @stellar imports.

### 6. Updated XDR parsing tasks (0024–0027)

- Removed `related_tasks` reference to archive task 0013 (shared TS XDR lib, now obsolete)
- Added `related_adr: ['0004']` (Rust-only XDR parsing ADR)
- Added `rust` tag
- Added history note: "Scope changed to Rust-only per ADR 0004"

### 7. Updated architecture docs

- `docs/architecture/xdr-parsing/xdr-parsing-overview.md` — replaced "Two Parsing Paths" (ingestion + on-demand) with "Single Parsing Path — Rust at Ingestion Time". Replaced "API-Time Parsing Is Narrow and On-Demand" with "Raw XDR Passthrough for Advanced Views".
- `docs/architecture/backend/backend-overview.md` — changed "Raw XDR on demand" to "Raw XDR passthrough". Removed reference to `@stellar/stellar-sdk` in API. Changed "XDR decode helpers" to "raw XDR passthrough".

### 8. Created ADRs

- **ADR 0003** — Use milestone frontmatter field for deliverable ordering instead of renumbering task IDs (initial decision, later superseded by the full reindex)
- **ADR 0004** — Rust-only XDR parsing: eliminate TS on-demand decode. Extends ADR 0002 (Rust Ledger Processor).

### 9. Moved OpenAPI task (old 0038) to M2

Old 0038 (OpenAPI docs) was initially assigned M1 because D1 scope says "OpenAPI specification". But the task explicitly requires all API feature modules to exist ("depends on all feature module tasks"). Split into:

- **0042** (M1) — Swagger infrastructure setup (tooling, spec skeleton)
- **0057** (M2) — Full OpenAPI endpoint documentation (all 20+ endpoints annotated)

## Acceptance Criteria

- [x] All backlog tasks reindexed: IDs follow M1 → M2 → M3 order
- [x] IDs are sequential within each milestone (M1: 0016–0042, M2: 0043–0077+0086–0087, M3: 0088–0090)
- [x] All `related_tasks` across backlog, archive, and active updated to new IDs (0 dangling refs)
- [x] All tasks (backlog, archive, active) have `milestone: N` in frontmatter
- [x] Milestone assignments match Three-Milestone Delivery Plan from technical design §7.4
- [x] Missing D1 tasks created: 0041 (Galexie config), 0042 (Swagger infra)
- [x] Missing D3 tasks created: 0088 (tests), 0089 (load test), 0090 (security audit)
- [x] Obsolete task removed: old 0035 (TS XDR decode helpers)
- [x] Dead code removed: `libs/shared/src/xdr/`, XDR exports, `@stellar/stellar-sdk` deps
- [x] Shared lib rebuilt clean: `libs/shared/dist/` has no XDR artifacts
- [x] XDR parsing tasks (0024–0027) updated: rust tag, ADR 0004 ref, removed 0013 dependency
- [x] Architecture docs updated: xdr-parsing-overview, backend-overview
- [x] ADR 0003 created (milestone field decision)
- [x] ADR 0004 created (Rust-only XDR parsing)

## Notes

- ADR 0003 documents the initial decision to use milestone fields instead of renumbering. We later did the full reindex anyway, but the milestone field remains valuable as metadata.
- Error types in `libs/shared/src/errors.ts` are kept — they define parse error contracts that the Rust indexer will produce and the NestJS API will consume.
- `lore-framework-mcp` index generator does not yet read or sort by `milestone` — the field is consumed by humans and Claude, not by tooling.
