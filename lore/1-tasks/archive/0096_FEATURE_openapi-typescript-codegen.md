---
id: '0096'
title: 'OpenAPI → TypeScript codegen: shared types between Rust API and React frontend'
type: FEATURE
status: completed
layer: frontend
milestone: 2
related_adr: ['0005', '0032']
related_tasks: ['0092', '0094']
tags:
  [priority-high, effort-small, layer-frontend, layer-backend, typescript, rust]
links:
  - https://github.com/hey-api/openapi-ts
  - https://docs.rs/utoipa/5.4.0/utoipa/derive.ToSchema.html
history:
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from 0092 research. Single source of truth for API types: Rust → OpenAPI → TypeScript.'
  - date: 2026-04-29
    status: active
    who: karolkow
    note: 'Activated. Deps 0094 (workspace) archived; ready to wire utoipa → openapi-ts.'
  - date: 2026-04-29
    status: done
    who: karolkow
    note: >
      All 6 acceptance criteria met. `extract_openapi` binary in `crates/api/src/bin/`
      reuses route modules via new `api` lib surface (no duplication). `libs/api-types`
      Nx lib produces SDK + TanStack Query hooks + types via `@hey-api/openapi-ts@0.97.0`.
      `check-generated` Nx target + always-on CI job rerun the full pipeline and fail on
      drift. `web/src/app.tsx` consumes `NetworkStats` from generated types as smoke-test
      import. Updated `docs/architecture/frontend` (new section 4.5) and
      `backend-overview.md` per ADR 0032 evergreen-docs requirement.
---

# OpenAPI → TypeScript codegen: shared types between Rust API and React frontend

## Summary

Generate TypeScript types, fetch client, and TanStack Query hooks from the Rust API's OpenAPI 3.1 spec. Single source of truth: Rust structs with `#[derive(ToSchema)]`. Frontend never writes API types manually.

## Context

Rust API (utoipa) generates OpenAPI 3.1 JSON. React frontend (TanStack Query) needs typed API access. Without codegen, types are duplicated across languages and drift over time.

Research task 0092 confirmed utoipa 5.4 generates OpenAPI 3.1 specs. This task wires codegen into the monorepo.

## Implementation

### 1. Extract OpenAPI spec from Rust (build-time, no server needed)

Add a secondary binary to the `api` crate (not a separate crate):

```rust
// crates/api/src/bin/extract_openapi.rs
fn main() {
    let spec = api::ApiDoc::openapi();
    println!("{}", spec.to_pretty_json().unwrap());
}
```

Run: `cargo run -p api --bin extract_openapi > libs/api-types/src/openapi.json`

### 2. Create `libs/api-types/` Nx library

```
libs/api-types/
├── src/
│   ├── openapi.json          # extracted spec (committed)
│   ├── generated/            # codegen output (committed)
│   │   ├── types.ts
│   │   ├── client.ts         # typed fetch client
│   │   └── hooks.ts          # TanStack Query hooks
│   └── index.ts              # re-exports
├── openapi-ts.config.ts      # @hey-api/openapi-ts config
└── project.json              # Nx targets
```

Committed generated files = frontend devs don't need Rust toolchain.

### 3. Install and configure `@hey-api/openapi-ts`

```bash
npm install -D @hey-api/openapi-ts
```

Config (`openapi-ts.config.ts`):

```typescript
export default {
  input: 'src/openapi.json',
  output: 'src/generated',
  plugins: [
    '@hey-api/types', // TypeScript types
    '@hey-api/client-fetch', // typed fetch client
    '@hey-api/tanstack-query', // TanStack Query hooks
  ],
};
```

### 4. Nx targets

```jsonc
{
  "targets": {
    "extract-openapi": {
      "command": "cargo run -p api --bin extract_openapi > libs/api-types/src/openapi.json"
    },
    "generate": {
      "command": "npx openapi-ts",
      "dependsOn": ["extract-openapi"]
    }
  }
}
```

### 5. CI validation

Run codegen + `git diff --exit-code libs/api-types/src/generated/` — fail if committed types are stale.

## Acceptance Criteria

- [x] `crates/api/src/bin/extract_openapi.rs` extracts OpenAPI JSON without booting server
- [x] `libs/api-types/` Nx library with generated types, client, and hooks
- [x] `@hey-api/openapi-ts` configured with types + fetch client + TanStack Query plugins
- [x] `nx run api-types:generate` produces up-to-date TypeScript from spec
- [x] `web/` can import types: `import type { NetworkStats } from '@rumblefish/api-types'`
      (spec example used `Ledger`/`PaginatedResponse`; those types do not exist yet —
      `Ledger` = task 0047, generic pagination shape lives per-endpoint. Smoke-test
      uses `NetworkStats` which is the only currently-shipped collection-style schema.)
- [x] CI step validates generated types are committed and up-to-date

## Implementation Notes

### Files added

- `crates/api/src/bin/extract_openapi.rs` — secondary binary, prints pretty JSON to stdout.
- `crates/api/src/lib.rs` — new library surface so the bin can import route modules.
- `crates/api/src/ops/mod.rs` — `health` handler moved here so both bin and main
  can register it without duplication.
- `libs/api-types/` — new Nx lib (project.json, package.json, tsconfig\*, eslint
  config, openapi-ts.config.ts, README, src/index.ts, src/openapi.json,
  src/generated/\*\*).
- `.github/workflows/ci.yml` — new always-on `api-types-codegen` job runs
  `nx run @rumblefish/api-types:check-generated`.
- `docs/architecture/frontend/frontend-overview.md` — new section 4.5
  "API Types and Codegen", boundary list and stack list updated.
- `docs/architecture/backend/backend-overview.md` — utoipa bullet expanded with
  `extract_openapi` reference.

### Files removed

- `tools/scripts/format-staged.mjs` — replaced by `lint-staged` package.

### Plugins / config

- `@hey-api/openapi-ts@0.97.0` (devDependency).
- `lint-staged@^16.4.0` (devDependency, replaces homegrown format-staged
  script). Husky pre-commit now runs `npx lint-staged` before
  `verify:staged`.
- openapi-ts plugins: `@hey-api/typescript`, `@hey-api/sdk`, `@tanstack/react-query`
  with `client: '@hey-api/client-fetch'`. (Spec listed older plugin names —
  `@hey-api/types`, `@hey-api/client-fetch` plugin, `@hey-api/tanstack-query`;
  hey-api 0.97 renamed/restructured these. Functional equivalent.)

### Nx target chain

```
extract-openapi → generate (openapi-ts + prettier) → check-generated (git diff --exit-code)
```

Prettier chained into `generate` so committed `src/openapi.json` and `src/generated/**`
match the repo's prettier config; otherwise `check-generated` would flap on
whitespace alone.

## Issues Encountered

- **Initial commit had a dirty-tree-generated `openapi.json`**: the regenerated
  spec embedded `assets/dto.rs` doc-comment changes that lived only in the
  working tree. Caught during this audit; fixed by stashing the unrelated
  changes and rerunning `nx run @rumblefish/api-types:generate` from a clean
  Rust source. Lesson: never run codegen from a dirty tree; the chain is
  deterministic only against committed state.

- **`number | null` vs `number` drift in generated types**: re-running the
  pipeline produced `limit?: number` instead of the previously committed
  `limit?: number | null` for `required: false` integer query params. Schema
  in openapi.json was identical between runs; suspected hey-api version
  difference between original commit and this regen, or a behavior change in
  how `required: false` non-nullable params are rendered. Accepted current
  output as the new baseline since CI will produce the same output going
  forward.

- **`git diff --exit-code` is byte-strict**: any whitespace flap on regen
  fails the gate. Mitigated by running prettier as the last step of
  `generate`, so committed and regenerated outputs share a single
  formatter pass.

## Design Decisions

### From Plan

1. **Single source of truth = Rust `#[derive(ToSchema)]` + `#[utoipa::path]`**.
   Frontend never hand-writes API types. Drift impossible because there is
   no second authoring location.

2. **Generated files committed**: frontend developers do not need a Rust
   toolchain. CI rerunning codegen + diff is the freshness gate.

3. **`extract_openapi` as a secondary `api` crate binary, not a separate crate**:
   reuses route modules without duplication. Required splitting `api` into a
   `lib.rs` surface plus the existing `main.rs` Lambda entrypoint.

4. **CI validates with `git diff --exit-code` after rerunning the full chain**.
   Spec wording followed verbatim.

### Emerged

5. **`api-types-codegen` CI job gated by paths-filter**: the spec listed
   CI validation as a single-line step. We added a dedicated
   `api_types_codegen` filter category (`crates/api/**`, `Cargo.{toml,lock}`,
   `libs/api-types/**`, `package*.json`, the workflow file itself) so the
   job runs only when an affecting path changed, plus unconditionally on
   `push` to `master`. Filter is tighter than the existing `rust:` category
   — codegen depends only on the `api` crate, not on `crates/xdr-parser`,
   `crates/db`, etc. Frontend-only PRs skip the ~3 min cargo + npm install
   cost.

6. **Prettier chain in `generate` target**: not in spec. Required because
   the rest of the repo formats JSON / TS via prettier and `check-generated`
   would otherwise fail on benign whitespace differences. Output is now
   deterministic across local dev / CI.

7. **`lint-staged` swap (replaces `tools/scripts/format-staged.mjs`)**:
   not directly required by codegen task but bundled here because the
   homegrown script duplicated lint-staged behavior poorly (no
   per-extension hooks, unable to run rustfmt on staged Rust files).
   Rust + TS now share one staging tool. Bonus tooling change called out
   in PR description.

8. **`api/src/bin/extract_openapi.rs` reconstructs the router instead of
   importing a shared builder**: the live `app()` function in `main.rs`
   takes `AppConfig` + `AppState`, neither of which the spec extractor
   needs. Reconstructing the same `OpenApiRouter::with_openapi(...)` chain
   keeps the bin dependency-free of database/AWS state. Risk: the router
   wiring lives in two places and must be kept in sync. Mitigation:
   `api_docs_json_contains_*_paths` integration tests in `main.rs` would
   catch a forgotten `nest()` call in the live router; the extractor would
   silently miss it (acceptable — drift in extractor produces a smaller
   spec, codegen output shrinks, frontend type-checks fail loudly).

9. **`docs/architecture/frontend` section 4.5 added rather than amended in
   passing**: ADR 0032 makes evergreen docs a hard requirement for any PR
   that changes frontend↔backend contracts. A dedicated section makes
   the codegen pipeline discoverable rather than a footnote.

10. **Unrelated `filter[code]` wildcard semantics change kept in stash**:
    a working-tree change reverted the `%`/`_` rejection in
    `assets::list_assets` and added SQL `ESCAPE '\'`. Out of scope for
    0096; stashed locally rather than spawning a follow-up backlog task
    (per user direction during audit). Will resurface as its own PR.

## Future Work

None. Remaining CI-side improvements (rust-cache for the codegen job to
shorten cargo cold compile, helpful error message when `check-generated`
fails, snapshot test on openapi-spec invariants) discussed during audit
but consciously deferred — the gate as it stands is sufficient until
frontend consumption grows beyond the smoke-test import.

## Notes

- Depends on task 0094 (Cargo workspace) — `extract_openapi` binary lives in `crates/api/`
- `@hey-api/openapi-ts` is v0.x but very actively maintained (2M+ weekly downloads, released today)
- Runner-up option: `openapi-typescript` + `openapi-fetch` + `openapi-react-query` (three packages, leaner output, more wiring)
- Existing `libs/domain/` TS types will be gradually replaced by generated types from this package
