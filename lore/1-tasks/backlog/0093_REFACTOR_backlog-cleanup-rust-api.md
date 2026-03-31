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

### Step 4: Decide libs/database fate

Based on 0092 research:

- If Rust ORM has good migrations → remove Drizzle, update 0016-0020 schema tasks
- If not → keep Drizzle Kit as migration tooling alongside Rust query layer

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
