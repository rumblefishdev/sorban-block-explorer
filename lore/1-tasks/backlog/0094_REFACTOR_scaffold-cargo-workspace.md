---
id: '0094'
title: 'Scaffold Cargo workspace with 5 crates'
type: REFACTOR
status: backlog
related_adr: ['0005']
related_tasks: ['0092', '0024']
tags: [priority-high, effort-medium, layer-backend, rust, milestone-1]
links: []
history:
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from 0092 research. Blocks all Rust API/indexer implementation tasks.'
---

# Scaffold Cargo workspace with 5 crates

## Summary

Create the root Cargo workspace and migrate existing Rust code into it. This is the structural prerequisite for all Rust backend implementation — nothing can be built until the workspace exists.

## Context

Research task 0092 recommended 5 crates. Task 0024 already created `apps/indexer/crates/xdr-parser/` as a standalone crate (outside any workspace). This task creates the workspace and migrates xdr-parser into it.

## Implementation

1. Create root `Cargo.toml` with `[workspace]` and `[workspace.dependencies]`
2. Create `crates/` directory with 5 crate scaffolds:
   - `crates/api/` — binary, axum Lambda handler (empty main with health check)
   - `crates/indexer/` — binary, Ledger Processor Lambda (empty main)
   - `crates/xdr-parser/` — **migrate** from `apps/indexer/crates/xdr-parser/`
   - `crates/db/` — library, sqlx pool + migrations directory
   - `crates/domain/` — library, shared types (Ledger, Transaction structs)
3. Add `rust/project.json` with `nx:run-commands` targets (build, test, lint, fmt-check)
4. Move existing Drizzle migration to `crates/db/migrations/` (strip breakpoint comments)
5. Verify: `cargo check --workspace` passes
6. Update `.prettierignore`, `.gitignore` for `crates/*/target/`
7. Remove `tools/scripts/explore-xdr-rs/` (superseded by xdr-parser)
8. Remove `tools/scripts/api-stack-test/` (PoC served its purpose)

## Acceptance Criteria

- [ ] Root `Cargo.toml` with workspace members for all 5 crates
- [ ] `crates/xdr-parser/` migrated from `apps/indexer/crates/xdr-parser/` (code unchanged, tests pass)
- [ ] `crates/api/` scaffolded with axum + lambda_http + utoipa (minimal health endpoint)
- [ ] `crates/db/` with sqlx pool config + migrated Drizzle SQL
- [ ] `crates/domain/` with Ledger and Transaction structs (matching DB schema)
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes
- [ ] Nx `rust` project with build/test/lint targets works
