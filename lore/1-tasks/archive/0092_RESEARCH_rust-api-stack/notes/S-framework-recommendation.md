---
type: synthesis
status: mature
spawned_from: 0092
tags: [framework, decision, axum, utoipa]
---

# Synthesis: Framework Recommendation

**Decision: axum 0.8 + utoipa 5.4 (OpenAPI)**

## Verified Stack

| Crate       | Version | Verified                                                   |
| ----------- | ------- | ---------------------------------------------------------- |
| axum        | 0.8.8   | crates.io + cargo check                                    |
| lambda_http | 1.1.2   | crates.io (released 2026-03-19)                            |
| sqlx        | 0.8.6   | crates.io + cargo check                                    |
| utoipa      | 5.4.0   | crates.io + cargo check                                    |
| utoipa-axum | 0.2.0   | crates.io + cargo check (stable, compatible with axum 0.8) |
| tower-http  | 0.6.8   | crates.io + cargo check                                    |

All compile together (cargo check, edition 2024, rustc 1.94.0).

## Why axum (ranked by weight)

1. **Lambda integration is native.** axum Router → lambda_http::run() with zero adapter. Official AWS examples. lambda_http v1.1.2 released 12 days ago.

2. **Community and maintenance.** 25,477 stars, tokio team (tokio-rs org), two substantial contributors. Largest Rust web framework by adoption.

3. **Tower-native middleware.** Entire tower ecosystem available. CORS, tracing, compression, auth — all standard tower layers.

4. **Runtime compatibility.** Standard `#[tokio::main]`. Zero conflicts with stellar-xdr or any tokio-based crate. Ledger Processor is already tokio-based.

## Why utoipa over aide for OpenAPI

After deep-dive comparing CRUD patterns, cursor pagination, and documentation features:

**utoipa wins on documentation quality (5-2-5 vs aide):**

| utoipa wins                                 | aide wins                      | Tie                                                                   |
| ------------------------------------------- | ------------------------------ | --------------------------------------------------------------------- |
| Endpoint docs (doc comments auto-map)       | CRUD generics (no macros)      | Field descriptions                                                    |
| Inline examples (fields, params, responses) | Security schemes (cleaner API) | Cursor pagination                                                     |
|                                             |                                | Schema naming (both need concrete types; `#[aliases]` removed in 5.x) |
| Deprecation (`#[deprecated]` native)        |                                | Response codes                                                        |
| Tags (per-router batch via `nest()`)        |                                | Enums                                                                 |

**Key factors for our use case:**

1. **Public API documentation matters.** Frontend devs will use Swagger UI daily. utoipa's richer examples, clean schema names, and native deprecation make better docs.

2. **CRUD base pattern is solvable.** aide's generic advantage is real but `macro_rules!` in utoipa works. ~120 lines once, ~10 lines per resource. Total boilerplate is identical (~950 lines for 10 resources).

3. **aide's documentation gaps are hard to fix:**

   - No `#[deprecated]` → workaround via `inner_mut()`
   - No per-router tag batch → must tag each operation individually
   - Weak parameter examples → no inline param examples
   - Generic schema names ugly (`PaginatedResponse_Ledger`) → needs manual `JsonSchema` impl

4. **utoipa has 14x more downloads** (22.8M vs 1.6M). More examples, blog posts, SO answers.

5. **utoipa-axum is stable** (confirmed: not stale, feature-complete for axum 0.8.x).

## Cold Starts (Verified)

lambda-perf data (2026-03-30, provided.al2023, ARM64, zip):

- **Avg init: ~14ms** (range 12-19ms across 10 samples)
- **Memory: 15 MB**
- Binary: 5.6 MB stripped, 2.6 MB gzipped (macOS native; ARM64 cross-compile will be smaller)
- No provisioned concurrency needed

## CRUD Pattern

`macro_rules! crud_routes` generates concrete handlers with `#[utoipa::path]` per resource. Each resource:

- Model struct with `ToSchema` (~20 lines)
- `CrudResource` impl with DB queries (~45 lines)
- `crud_routes!` invocation (~10 lines)
- 1 alias in `PaginatedResponse`
- 1 `.merge()` in main router

Cursor pagination: `PaginationParams` with `IntoParams`, `PaginatedResponse<T>` with `#[aliases]`. Opaque base64 cursor documented via doc comments + `#[param(example)]`.

## What We Lose

- **aide's pure Rust generics for CRUD** — cleaner than macros but not worth the documentation trade-offs.
- **poem-openapi's fully integrated DX** — best OpenAPI DX of all, but bus factor risk (83% single maintainer).

## Risk Factors to Monitor

- **utoipa open issues (199)** — high count but maintainer is active (commits Feb 2026). utoipa-axum is stable.
- **axum 0.9** — breaking changes in progress on main. Pin to 0.8.x.
- **Fallback:** aide 0.15.1 is cargo-check verified as backup. Migration: swap derives + rewrite transform functions.

## Next Step

Proceed to ORM/database research (Q8-Q14) with axum + utoipa as the confirmed framework + OpenAPI choice.
