---
id: '0042'
title: 'OpenAPI/Swagger infrastructure setup'
type: FEATURE
status: completed
related_adr: ['0005', '0008']
related_tasks: ['0023', '0057', '0092']
tags: [priority-medium, effort-small, layer-backend]
milestone: 1
links: []
history:
  - date: 2026-03-30
    status: backlog
    who: fmazur
    note: 'Task created — split from 0057 during milestone alignment (task 0085). D1 requires OpenAPI specification infrastructure; full endpoint documentation is M2 (task 0057).'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: NestJS → Rust (axum + utoipa + sqlx)'
  - date: 2026-04-08
    status: active
    who: stkrolikiewicz
    note: 'Activated — dependency 0023 (API bootstrap) completed; starting M1 OpenAPI infra.'
  - date: 2026-04-08
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped M1 OpenAPI scaffold. 6 commits, 7 files, ~450 LoC.
      utoipa-swagger-ui gated behind opt-in `swagger-ui` feature
      after hard measurement: default build 4.35 MB (budget 5 MB),
      `--features swagger-ui` 15.44 MB (budget 17 MB). ADR 0008
      cements ErrorEnvelope + cursor-based Paginated<T> shapes for
      all M2 endpoints. 4 unit tests × 2 feature configs all green.
---

# OpenAPI/Swagger infrastructure setup

## Summary

Set up utoipa OpenAPI integration, document builder configuration, utoipa-swagger-ui dev endpoint, and spec export pipeline. This is the M1 infrastructure prerequisite for the full OpenAPI endpoint documentation (task 0057, M2). D1 design scope includes "OpenAPI specification" — this task delivers the tooling and empty spec skeleton; task 0057 fills it with all 20+ endpoint annotations.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Backlog

**Current state:** Not started. Depends on task 0023 (API bootstrap).

## Context

The technical design (§7.4 D1) lists "OpenAPI specification" in the D1 scope. However, the full spec (task 0057) requires all API feature modules (M2) to exist. This task splits out the infrastructure part that can be delivered in M1: utoipa setup, document builder, dev UI, and export pipeline.

## Implementation Plan

### Step 1: Install and configure utoipa + utoipa-swagger-ui

Add `utoipa` and `utoipa-swagger-ui` dependencies. Configure `OpenApi` derive macro with API title, description, version, base URL, and contact info in `crates/api/src/`.

### Step 2: Define reusable schema components

Create shared OpenAPI schema components (via `ToSchema` derive) for: error envelope, pagination envelope, standard query parameters. These components will be referenced by endpoint annotations in task 0057.

### Step 3: Swagger UI dev endpoint

Configure utoipa-swagger-ui at `/api-docs` in development/staging environments. Ensure it is disabled in production.

### Step 4: Spec export as JSON

Set up OpenAPI spec export as JSON at `/api-docs-json` via axum route.

## Acceptance Criteria

- [x] `utoipa` and `utoipa-swagger-ui` configured in the API crate (latter behind opt-in `swagger-ui` feature)
- [x] OpenApi derive configured with API metadata — title, `version = env!("CARGO_PKG_VERSION")`, description, contact
- [x] Reusable schema components defined via `ToSchema` — `ErrorEnvelope`, `PageInfo`, `Paginated<T>` (all in `crates/api/src/openapi/schemas.rs`, cemented by ADR 0008)
- [x] Swagger UI available at `/api-docs` when built with `--features swagger-ui`
- [x] OpenAPI spec exportable as JSON at `/api-docs-json` — always on, regardless of feature
- [x] Swagger UI and spec JSON served directly from the API (no S3 publication pipeline)
- [x] Hard size budgets enforced: default build ≤ 5 MB (measured 4.35 MB), `--features swagger-ui` ≤ 17 MB (measured 15.44 MB)
- [x] CI validates both feature configurations — `cargo clippy`/`cargo test` steps for default and `--features swagger-ui`
- [x] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` green under both feature configurations
- [x] ADR 0008 published alongside this task

## Implementation Notes

Six commits on `feat/0042_openapi-swagger-infrastructure`:

1. **`chore(lore-0042): activate task`** — promote + history entry.
2. **`chore(lore-0042): add utoipa-swagger-ui as optional feature-gated dep`** — workspace `utoipa-swagger-ui 9` (verified via `cargo info`, paired with `utoipa 5.4`), `crates/api/Cargo.toml` declares `[features] default = []; swagger-ui = ["dep:utoipa-swagger-ui"]`.
3. **`feat(lore-0042): wire OpenApiRouter with schemas and feature-gated Swagger UI`** — 4 new files (`config.rs`, `openapi/mod.rs`, `openapi/schemas.rs`, updated `main.rs`). `AppConfig::from_env()` is the only env-reading code path; `fn app(&AppConfig)` is pure. Uses `utoipa_axum::router::OpenApiRouter::routes(routes!(health))` for auto-registration; `mount_swagger_ui` is `#[cfg(feature = "swagger-ui")]` gated so production binary never links the ~12 MB embedded assets. `SwaggerUi::new("/api-docs").url("/api-docs/openapi.json", spec)` uses a dedicated internal URL to avoid colliding with the always-on public `/api-docs-json`.
4. **`feat(lore-0042): pass API_BASE_URL to api Lambda from compute-stack`** — adds `API_BASE_URL: https://${config.apiDomainName}` to `apiFunction.environment` so the OpenAPI `servers` block reflects the deploy environment.
5. **`ci(lore-0042): validate api crate under both default and swagger-ui features`** — extra clippy + test steps in `.github/workflows/ci.yml` guard against bit-rot in the UI-enabled build path.
6. **`docs(lore-0008): ADR — error envelope and pagination shape`** — cements `ErrorEnvelope {code, message, details}` and cursor-based `Paginated<T>` as the load-bearing M2 contract.

**Tests** (`crates/api/src/main.rs`, all via `tower::ServiceExt::oneshot` against `app(&test_config())`):

1. `health_returns_ok` — regression guard, existing
2. `api_docs_json_contains_health_path` — verifies spec contains `/health`, info.title, info.version from Cargo, and runtime-stamped `servers[0].url`
3. `api_docs_json_has_error_envelope_component` — asserts `ErrorEnvelope` and `PageInfo` schemas exposed in `components`
4. `swagger_ui_mounted_when_feature_enabled` (#[cfg(feature = "swagger-ui")]) — `/api-docs/` returns 2xx/3xx
5. `swagger_ui_absent_without_feature` (#[cfg(not(feature = "swagger-ui"))]) — `/api-docs/` returns 404

All tests green under both feature configurations (4 tests × 2 configs = 8 verifications).

## Design Decisions

### From Plan

1. **`utoipa-swagger-ui` gated behind Cargo feature, not runtime env check**: Compile-time gating means the production Lambda binary never pulls in the embedded UI assets at all. Measured impact: 4 MB → 15 MB when the UI is linked (4× binary growth). Runtime `ENV_NAME` check would have still paid the 12 MB cost in production. Feature flag avoids it.

2. **`default = []` (feature OFF by default)**: Prod-safe baseline. Local `cargo run -p api --features swagger-ui` is one extra flag but removes the failure mode where a prod build forgets `--no-default-features` and ships a 15 MB binary. DX cost is ~5 characters per dev command, acceptable.

3. **`AppConfig` + `fn app(&AppConfig) -> Router`**: All `std::env` reads live in `AppConfig::from_env()`. Tests construct `AppConfig` directly, never touch process env. Avoids `serial_test` + env mutation complexity.

4. **`OpenApiRouter::routes(routes!(handler))` paradigm, not `#[derive(OpenApi)] paths(...)` listing**: utoipa 5 supports both. Auto-registration via `routes!` scales to M2's 20+ endpoints without manual list maintenance.

5. **`env!("CARGO_PKG_VERSION")` at the `ApiDoc` derive site**: Single source of truth — bumping the crate version in `Cargo.toml` automatically updates the advertised API version. No drift possible.

6. **Runtime `servers` block from `AppConfig.base_url`**: The registered spec (post `split_for_parts`) gets its `servers` field stamped with `AppConfig.base_url` so the same binary advertises `http://localhost:9000` locally, `https://api.staging.sorobanscan.rumblefish.dev` on staging, etc.

### Emerged

7. **`ErrorEnvelope` shape over RFC 7807**: Considered formalising with problem+json but rejected — see ADR 0008 alternatives section. Three-field envelope is strictly enough and avoids the `type` URI maintenance burden.

8. **Dedicated internal URL for SwaggerUi spec (`/api-docs/openapi.json`)**: First wire-up tried `SwaggerUi::new("/api-docs").url("/api-docs-json", spec)` which collided with the always-on public `/api-docs-json` handler registered one step earlier (`Overlapping method route` panic in test). Split by giving SwaggerUi its own internal URL while keeping the public one canonical.

9. **`ApiDoc::components(schemas(ErrorEnvelope, PageInfo))` not `Paginated<T>`**: utoipa 5 `ToSchema` on generics requires concrete instantiation at the handler level — no concrete `Paginated<SomeType>` exists yet in M1, so listing it in `components(...)` would fail compilation. Instead, `Paginated<T>` lives as a type ready for M2 handlers to pick up; concrete instantiations will register automatically as handlers are added.

10. **`#[allow(dead_code)]` on `Paginated<T>`**: Struct is infrastructure waiting for M2 consumers, so unused in M1 by design. Tagged explicitly rather than letting clippy warn.

11. **Local `NX_SOCKET_DIR=/tmp/nx-tmp NX_DAEMON=false` workaround**: macOS socket path length limit (~104 bytes) caused husky pre-commit hooks to fail via `nx graph`. Workaround applied at commit time; underlying nx issue is an environmental gotcha not fixable in this task. Flagged as developer environment note.

12. **Dropped `AppConfig.api_version` field**: Initial design had it as a runtime constant, but since the value is always `env!("CARGO_PKG_VERSION")` — which can go directly into the `ApiDoc` derive — the field was redundant. Removed to keep `AppConfig` focused on truly runtime-configurable values.

## Issues Encountered

- **`ApiDoc::openapi()` not in scope after refactor**: Moved imports around and forgot to bring `utoipa::OpenApi` trait into scope, so `ApiDoc::openapi()` wouldn't resolve. `rustc` told me exactly which trait to import. One-line fix.

- **`SwaggerUi::url` collided with manually mounted `/api-docs-json`**: First try had both SwaggerUi's embedded JSON handler and my explicit `/api-docs-json` route on the same path, which axum rejects with `Overlapping method route`. Fixed by giving SwaggerUi a dedicated internal path (`/api-docs/openapi.json`) and keeping the public one canonical.

- **ID collision near-miss during activation**: Local `develop` had an unpushed `chore(lore-0042): activate task` commit that had been created by `/promote-task` but blocked from pushing by the user. After the user merged a batch of tasks on remote `develop`, rebase put the activation commit onto the branch cleanly instead of needing a manual port. Worth documenting as a gotcha: `/promote-task` outputs that are not actually pushed need manual tracking.

- **Initial dev branch had local-develop-based activate commit**: Branch was forked from a local develop that included the unpushed activate commit. `git rebase origin/develop` cleanly replayed all 6 commits (activate + 5 features) onto the fresh remote develop.

## Future Work

- **`Paginated<T>` concrete registrations in M2**: Each endpoint module landing in tasks 0043–0053 will need to register its own concrete `Paginated<SomeType>` so utoipa emits the schema. Boilerplate, not scope creep.
- **Consider publishing the OpenAPI spec as a build artifact**: Task 0057 may want a pre-generated `openapi.json` in the repo or S3 for external SDK generation. Not scoped here.
- **Nx macOS socket path workaround**: Add `.envrc` / direnv or repo-level `NX_SOCKET_DIR` default so new contributors do not trip on the same issue.

## Notes

- This task delivers the "OpenAPI specification" infrastructure required by D1.
- Task 0057 (M2) depends on this and adds full endpoint annotations after all API modules are built.
- ADR 0008 published alongside to cement the shapes as a cross-task contract.
