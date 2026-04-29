# api-types

OpenAPI-derived TypeScript types, fetch SDK, and TanStack Query hooks
for the backend REST API. The Rust API crate is the source of truth;
artifacts under `src/openapi.json` and `src/generated/**` are
regenerated from it.

## Targets

- `nx run @rumblefish/api-types:extract-openapi` — dump the OpenAPI
  spec from the live `extract_openapi` Rust binary into
  `src/openapi.json`.
- `nx run @rumblefish/api-types:generate` — re-run codegen
  (`@hey-api/openapi-ts`) from `src/openapi.json` and prettier-format
  the result. Depends on `extract-openapi`.
- `nx run @rumblefish/api-types:check-generated` — fail if the
  committed artifacts disagree with what `generate` produces. Used by
  CI to catch drift.
- `nx run @rumblefish/api-types:build` — Nx-inferred TypeScript build
  (`tsc --build tsconfig.lib.json`).

Whenever a Rust DTO with `ToSchema` or a handler with `#[utoipa::path]`
changes, run `generate` and commit the updated artifacts.
