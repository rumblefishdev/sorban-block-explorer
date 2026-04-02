---
id: '0095'
title: 'Monorepo restructure: flatten apps/, infra/, move web to top-level'
type: REFACTOR
status: completed
related_adr: ['0005']
related_tasks: ['0092', '0094']
tags: [priority-medium, effort-small, layer-infra, nx]
links: []
history:
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from 0092 research. Independent of Rust implementation, can be done anytime.'
  - date: 2026-04-02
    status: active
    who: fmazur
    note: 'Activated task for implementation.'
  - date: 2026-04-02
    status: done
    who: fmazur
    note: >
      Monorepo restructured: web/ moved to top-level, infra/ flattened,
      apps/ removed, obsolete libs cleaned up. All Nx targets passing.
      CI criterion deferred.
---

# Monorepo restructure: flatten apps/, infra/, move web to top-level

## Summary

After Rust backend replaces TypeScript stubs, `apps/` has only `web/` and `infra/` has only `aws-cdk/`. Both are unnecessary wrapper directories. Restructure for clarity.

## Context

Post-0094 (Cargo workspace scaffold), the Rust backend lives in `crates/`. The remaining TypeScript projects (`web`, `aws-cdk`) each sit alone in a wrapper directory. Flattening improves navigation and removes dead directories.

## Implementation

1. `git mv apps/web/ web/` — React frontend to top-level
2. Flatten `infra/aws-cdk/` → `infra/` (move contents up one level, remove `aws-cdk/` subdirectory)
3. Remove `apps/api/` (NestJS, replaced by `crates/api/`)
4. Remove `apps/indexer/` (TS stub + old xdr-parser location, replaced by `crates/indexer/` + `crates/xdr-parser/`)
5. Remove `apps/workers/` (TS stub, replaced by Rust)
6. Remove empty `apps/` directory
7. Update Nx config: project paths in `nx.json`, `tsconfig.json` references
8. Update `package.json` workspaces if using npm/pnpm workspaces
9. Update CDK import paths (if referencing `../apps/...`)
10. Update CI workflow paths (`.github/workflows/`)
11. Update any docs referencing old paths

12. Remove `libs/database/` (Drizzle ORM, replaced by Rust sqlx in `crates/`)
13. Remove `libs/domain/` (TS domain types, replaced by `crates/domain/`)
14. Remove `libs/shared/` (TS error types, replaced by Rust error handling)
15. Update `tsconfig.json`, `package.json` workspaces after libs cleanup
16. Remove Drizzle scripts from root `package.json` (db:generate, db:migrate, db:studio)

## Acceptance Criteria

- [x] `web/` at top-level, `nx build web` works
- [x] `infra/` contains CDK directly (no `aws-cdk/` subdirectory)
- [x] `apps/` directory removed entirely
- [x] `libs/` contains only `ui/`
- [x] All Nx targets pass (`nx run-many -t lint build typecheck test`)
- [ ] CI pipeline passes

## Notes

- This is independent of Rust implementation — can be done before or after API is built
- Should be done AFTER task 0094 (scaffold Cargo workspace) to avoid moving files twice
- After ADR 0005 (Rust-only backend), `libs/database`, `libs/domain`, `libs/shared` are obsolete — no active imports
