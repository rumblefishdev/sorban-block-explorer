---
id: '0093'
title: 'Backlog cleanup: cancel NestJS tasks, update CDK and indexer tasks for Rust API'
type: REFACTOR
status: backlog
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
---

# Backlog cleanup: cancel NestJS tasks, update CDK and indexer tasks for Rust API

## Summary

Systematically update the backlog after ADR 0005 (Rust-only backend). Cancel or supersede NestJS-specific tasks, update CDK tasks to reference Rust API Lambda, and align remaining backend tasks with the Rust stack chosen in research task 0092.

## Status: Backlog

**Current state:** Not started. Partially blocked by 0092 — need to know Rust ORM choice before deciding fate of Drizzle/libs/database tasks.

## Implementation Plan

### Step 1: Cancel NestJS-specific tasks

Tasks that are NestJS-only and have no Rust equivalent:

- 0023 (NestJS bootstrap) — already completed/archived, mark as superseded by ADR 0005
- Any NestJS-specific middleware, DI, or decorator tasks

### Step 2: Rewrite backend module tasks for Rust

Tasks 0043-0055 (pagination, validation, backend modules, caching, OpenAPI) were written for NestJS. Rewrite scope descriptions for Rust framework chosen in 0092. Same endpoints, different implementation.

### Step 3: Update CDK tasks

- 0033 (CDK Lambda/API Gateway) — update to reference Rust API binary via cargo-lambda-cdk (not Node.js Lambda)
- Remove NestJS-specific CDK constructs (nestjs-lambda.ts)

### Step 4: Update DB schema tasks — Drizzle → sqlx plain SQL

Research 0092 decided: **sqlx migrations (plain SQL), drop Drizzle Kit.** Task 0017 already updated as reference. Update these:

| Task                              | Change                                                                                                                                        |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| **0018** (Soroban tables)         | Replace "Drizzle schema" steps → "plain SQL migration". Remove `.ts` schema step. Add: run via `psql`.                                        |
| **0019** (tokens, accounts)       | Same as 0018.                                                                                                                                 |
| **0020** (NFTs, pools, snapshots) | Same as 0018.                                                                                                                                 |
| **0021** (migration framework)    | **Supersede** — Drizzle Kit framework replaced by sqlx. Rewrite as: sqlx-cli setup, `sqlx::migrate!()` in binary, CI `sqlx migrate run` step. |
| **0022** (partition management)   | No change — already PG-native, no Drizzle dependency.                                                                                         |

Per task, apply these edits:

- "Drizzle ORM schema definition" → "Plain SQL migration file"
- "Drizzle Kit generate" → "Write SQL in `crates/db/migrations/`"
- Remove all `.ts` schema file steps
- Add `related_adr: ['0005']` and `related_tasks: ['0092']`
- Add note: "Run via `psql` or `sqlx migrate run`, not Drizzle Kit"
- Target format: plain `.sql` file in `crates/db/migrations/` (after 0094 creates workspace). Pre-0094 migrations go to `libs/database/drizzle/` as transitional location.

**`libs/database/` fate:** Stays until task 0094 migrates SQL files to `crates/db/migrations/`. Then archive (Drizzle TS schema files = dead code, frontend uses `libs/api-types` for types).

### Step 5: Update docs/architecture

- `backend-overview.md` — NestJS → Rust framework
- `infrastructure-overview.md` — Node.js API Lambda → Rust API Lambda
- `technical-design-general-overview.md` — backend stack section

### Step 6: Wiki snapshot

Create `lore/3-wiki/project/architecture-snapshot-rust-backend.md` with current state after all changes.

## Acceptance Criteria

- [ ] All NestJS-only tasks canceled or superseded with reason
- [ ] Backend module tasks (0043-0055) updated for Rust
- [ ] CDK tasks updated to reference Rust API Lambda
- [ ] libs/database fate decided and documented
- [ ] docs/architecture updated
- [ ] Wiki snapshot created
- [ ] Lore index regenerated

## Notes

- Do NOT cancel tasks that are framework-agnostic (cursor pagination logic, search algorithms, etc.) — rewrite scope for Rust.
- apps/api NestJS code can be removed after Rust API is scaffolded (separate task).
- Frontend tasks (0058-0090) are unaffected.
