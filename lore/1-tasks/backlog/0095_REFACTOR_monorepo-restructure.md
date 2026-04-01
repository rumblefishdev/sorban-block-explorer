---
id: '0095'
title: 'Monorepo restructure: flatten apps/, infra/, move web to top-level'
type: REFACTOR
status: backlog
related_adr: ['0005']
related_tasks: ['0092', '0094']
tags: [priority-medium, effort-small, layer-infra, nx]
links: []
history:
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from 0092 research. Independent of Rust implementation, can be done anytime.'
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

## Acceptance Criteria

- [ ] `web/` at top-level, `nx build web` works
- [ ] `infra/` contains CDK directly (no `aws-cdk/` subdirectory), `npx cdk synth` works
- [ ] `apps/` directory removed entirely
- [ ] All Nx targets pass (`nx run-many -t lint build typecheck test`)
- [ ] CI pipeline passes

## Notes

- This is independent of Rust implementation — can be done before or after API is built
- Should be done AFTER task 0094 (scaffold Cargo workspace) to avoid moving files twice
- `libs/` stays as-is (domain, shared, ui used by web frontend)
