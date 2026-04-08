---
id: '0109'
title: 'BUG: hotfixes for 3 P1 issues found by Codex review on develop'
type: BUG
status: completed
related_adr: []
related_tasks: ['0029', '0067', '0099']
tags: [priority-high, effort-small, layer-infra, layer-frontend]
links: []
history:
  - date: 2026-04-07
    status: active
    who: stkrolikiewicz
    note: >
      Codex review on PR #70 (fix/0106) flagged 3 P1 bugs already on
      develop, unrelated to 0106 itself. All three are blockers for
      downstream work and need to land before more features.
  - date: 2026-04-08
    status: completed
    who: stkrolikiewicz
    note: >
      All 3 bugs fixed in dedicated commits. Bug 1 (ESM __dirname)
      verified end-to-end: `nx dev ui` reaches `VITE ready in 181 ms`
      on Node 22, zero ReferenceError. Bug 2 (indexer env vars)
      verified with `cargo check -p indexer` and exhaustive grep —
      no stale refs remain. Bug 3 (tsconfig include) verified with
      `nx typecheck infra` after `nx reset`. 4 files changed,
      +11/-7 net.
---

# BUG: hotfixes for 3 P1 issues found by Codex review on develop

## Summary

While running `/codex:review` on PR #70 (the SPA cache TTL fix), Codex inspected the entire branch diff against master and found three P1 issues that were already merged to develop in earlier tasks. None of them are caused by 0106 — they are pre-existing bugs that block downstream tasks. Hot-fix sweep before continuing with feature work.

## The Three Bugs

### Bug 1 — `__dirname` in ESM Vite configs (blocks frontend build)

`web/vite.config.ts:5` and `libs/ui/vite.config.ts:5` use `__dirname` at module top-level. The workspace `package.json` has `"type": "module"`, so Vite loads these configs as ESM and `__dirname` is undefined at runtime → `ReferenceError: __dirname is not defined`. `nx dev` and `nx build` for both projects fail before Vite can start.

**Blocks:** all UI/frontend tasks (0058–0086).

**Fix:** ESM equivalent — `dirname(fileURLToPath(import.meta.url))`.

### Bug 2 — Indexer Lambda env var name mismatch (broken in production)

`crates/indexer/src/main.rs:24-27` reads `DB_SECRET_ARN` and `RDS_ENDPOINT` env vars. But `infra/src/lib/stacks/compute-stack.ts:86-89` (and the rest of the project — `db-migrate`, `db-partition-mgmt`) sets `SECRET_ARN` and `RDS_PROXY_ENDPOINT`. The indexer Lambda will fail every cold start in any deploy that doesn't provide `DATABASE_URL` directly, which is the normal CDK path.

**Blocks:** indexer running on staging/production. Critical for production launch.

**Fix:** rename in indexer crate to match the rest of the project (`SECRET_ARN`/`RDS_PROXY_ENDPOINT`). The other naming was introduced by task 0029 in isolation, missing the existing convention from `db-migrate` and `db-partition-mgmt`.

### Bug 3 — `envs/*.json` in tsconfig include outside rootDir (breaks typecheck)

`infra/tsconfig.lib.json` sets `rootDir: "src"` but `include` lists `envs/**/*.json`, which is a sibling of `src`. `tsc` rejects with `TS6059: File is not under rootDir`. Currently masked because the project's typecheck target is cached in nx, but a fresh build or nx target rerun fails.

**Blocks:** infra typecheck, CI workflow (0039) on any PR touching infra.

**Fix:** remove `envs/**/*.json` from include. The bin scripts (`bin/staging.ts`, `bin/production.ts`) load envs via runtime `createRequire(import.meta.url)('../../envs/staging.json')`, not via TypeScript JSON import — so tsc doesn't need to see them.

## Acceptance Criteria

- [x] `nx dev web` and `nx dev @rumblefish/soroban-block-explorer-ui` start without `ReferenceError` — ui server reached `VITE ready in 181 ms` on Node 22
- [x] `cargo check -p indexer` passes; CDK env vars in compute-stack match what indexer reads
- [x] `nx typecheck @rumblefish/soroban-block-explorer-aws-cdk` passes after `nx reset`
- [x] No regression in existing synth or build targets (`nx build infra`, `nx build web`, `nx build ui` all green)

## Implementation Notes

Three dedicated commits on `fix/0109_develop-codex-review-hotfixes`:

- **`35cc3ab` — bug 1 (ESM `__dirname`)**: `libs/ui/vite.config.ts`, `web/vite.config.ts`. Inline `dirname(fileURLToPath(import.meta.url))` in the `root` option. +6/-2.
- **`2339029` — bug 2 (indexer env var names)**: `crates/indexer/src/main.rs`. Renamed reads `DB_SECRET_ARN` → `SECRET_ARN`, `RDS_ENDPOINT` → `RDS_PROXY_ENDPOINT`, plus matching error messages. +4/-4.
- **`0f5bb02` — bug 3 (tsconfig include)**: `infra/tsconfig.lib.json`. Dropped `envs/**/*.json` from `include`. +1/-1.

All three fixes target root cause — no workarounds, no new dependencies, no new abstractions.

## Design Decisions

### From Plan

1. **ESM idiom for bug 1**: `dirname(fileURLToPath(import.meta.url))` is the canonical Node ESM replacement for `__dirname` since Node 10. Preferred inline use in the `root` option (vs. a top-level const) to keep the diff minimal and match the existing style.

2. **Rename indexer env vars, not CDK**: Bug 2 fixed in the indexer crate (the lone dissenter) rather than in compute-stack, because `db-migrate` and `db-partition-mgmt` already use `SECRET_ARN`/`RDS_PROXY_ENDPOINT` — task 0029 introduced the mismatched names in isolation and missed the convention.

3. **Drop the envs glob from tsconfig, not move rootDir**: Bug 3 resolved by removing the `envs` glob from `infra/tsconfig.lib.json` include, not by widening `rootDir` to the infra root. Bin scripts load envs via runtime `createRequire`, so tsc never needs to see them — the glob was leftover from an earlier direct-import approach.

### Emerged

4. **Grep before/after for stale refs (bug 2)**: Ran an exhaustive grep across the entire repo for `DB_SECRET_ARN` / `RDS_ENDPOINT\b` outside of archive task notes to confirm no callers were missed (docs, .env files, tests, CI). Zero hits in live code.

5. **Node 22 required for local `nx dev` verification**: The AC check for `nx dev ui` could only be literally verified after switching to Node 22.22.0 (project `.nvmrc`). Node 18 throws a separate `crypto.hash is not a function` in Vite 7 that masks any success indicator. Not in scope to fix, but flagged as a general developer-onboarding gotcha.

## Issues Encountered

- **ID collision during activation**: Initial task was numbered 0107, but while working on the fix branch, `origin/develop` had already merged `fix/0107_api-custom-domain-dns-resolution` + `feat/0108_galexie-ecs-health-check`. Local lore index was stale (no `git pull`) and didn't see the conflict. Renumbered to 0109, rebased branch on fresh develop, regenerated index. Future sessions: always `git pull origin develop` before assigning a new task ID.

- **Node version mismatch masks bug 1 fix on local dev**: First `nx dev ui` run on Node 18.19.0 failed with `TypeError: crypto.hash is not a function` (Vite 7 requirement: Node 20.19+/22.12+). Resolved by switching to Node 22.22.0 per project `.nvmrc`. The underlying ESM `__dirname` fix was correct from the start.

## Future Work

- **Harmonize `.env.example`**: Line 3 has `DATABASE_SECRET_ARN=` — a third unused name for the same thing (no code reads it). Out of scope but worth a follow-up cleanup task.
- **Switch vite configs to `node:` prefix imports**: Cosmetic senior-convention improvement (`'node:path'` / `'node:url'`). Out of scope, kept consistent with existing style.

## Notes

- Each bug fixed in its own commit so individual fixes can be reverted/cherry-picked
- All three are pre-existing on develop, not introduced by this task or by 0106
- Spawned from /codex:review run on PR #70
- Originally numbered 0107 locally; renumbered to 0109 to resolve ID collision with already-merged tasks
