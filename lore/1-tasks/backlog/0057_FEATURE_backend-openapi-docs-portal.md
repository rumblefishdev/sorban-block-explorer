---
id: '0057'
title: 'Backend: OpenAPI spec generation and docs portal'
type: FEATURE
status: backlog
related_adr: ['0005']
related_tasks: ['0023', '0042', '0092']
tags: [layer-backend, openapi, documentation, swagger]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: axum → Rust (axum + utoipa + sqlx)'
---

# Backend: OpenAPI spec generation and docs portal

## Summary

Integrate `utoipa` to auto-generate an OpenAPI specification from axum decorators. The spec documents all 20+ endpoints including query parameters, response schemas, error envelopes, filter parameters, pagination format, and cache-control behavior. utoipa-swagger-ui is served directly from the API (axum `/api-docs` endpoint).

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Backlog

**Current state:** Not started. Depends on task 0023 (API bootstrap) and all feature module tasks.

## Context

A comprehensive OpenAPI specification serves both as interactive documentation for API consumers and as a machine-readable contract for frontend integration, testing, and third-party tooling. The spec must be auto-generated from the actual axum code to stay in sync with the implementation.

### API Specification

**OpenAPI generation:** `utoipa` decorators on controllers, request/response types, and response types.

**Publication target:** utoipa-swagger-ui served directly from the API (axum `/api-docs` endpoint). No separate S3 + CloudFront setup.

### Documented Endpoints (20+ total)

| Module          | Endpoints                                                                                                                                               |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Network         | `GET /network/stats`                                                                                                                                    |
| Transactions    | `GET /transactions`, `GET /transactions/:hash`                                                                                                          |
| Ledgers         | `GET /ledgers`, `GET /ledgers/:sequence`                                                                                                                |
| Accounts        | `GET /accounts/:account_id`, `GET /accounts/:account_id/transactions`                                                                                   |
| Tokens          | `GET /tokens`, `GET /tokens/:id`, `GET /tokens/:id/transactions`                                                                                        |
| Contracts       | `GET /contracts/:contract_id`, `GET /contracts/:contract_id/interface`, `GET /contracts/:contract_id/invocations`, `GET /contracts/:contract_id/events` |
| NFTs            | `GET /nfts`, `GET /nfts/:id`, `GET /nfts/:id/transfers`                                                                                                 |
| Liquidity Pools | `GET /liquidity-pools`, `GET /liquidity-pools/:id`, `GET /liquidity-pools/:id/transactions`, `GET /liquidity-pools/:id/chart`                           |
| Search          | `GET /search`                                                                                                                                           |

### Spec Must Document

1. **All endpoints** with method, path, and description
2. **Query parameters** with types, validation rules, defaults, and allowed values
3. **Path parameters** with types and format descriptions
4. **Response schemas** for all success responses (200)
5. **Error envelopes** for all error responses (400, 404, 500)
6. **Filter parameters** with allowed values per endpoint
7. **Pagination format** (cursor-based, standard envelope)
8. **Cache-control behavior** per endpoint (documented in description)
9. **Dual-mode transaction detail** (normal vs advanced via `?view=advanced`)

### Response Shape (OpenAPI spec endpoint)

The generated spec is served as a JSON/YAML file:

```
GET /api-docs          -> utoipa-swagger-ui (interactive docs)
GET /api-docs-json     -> OpenAPI spec as JSON
```

### Example utoipa Annotations

```rust
#[utoipa::path(
    get,
    path = "/transactions",
    tag = "Transactions",
    summary = "List transactions",
    params(PaginationParams),
    responses(
        (status = 200, description = "Paginated list", body = PaginatedTransactions),
        (status = 400, description = "Invalid cursor", body = ErrorBody),
    )
)]
async fn list_transactions(...) -> Result<Json<PaginatedTransactions>, AppError> { ... }
```

### Behavioral Requirements

- Spec auto-generated from `#[utoipa::path]` and `#[derive(ToSchema)]` annotations (not manually maintained)
- All request/response types derive `ToSchema` with field-level `#[schema(...)]` annotations
- Error envelope documented as reusable schema component
- Pagination envelope documented as reusable schema component
- utoipa-swagger-ui available in development for interactive testing
- utoipa-swagger-ui and spec JSON served directly from the API

### Caching

- Published spec is a static file; long-TTL caching on CloudFront is appropriate.
- utoipa-swagger-ui in development has no caching.

### Error Handling

- Spec generation errors should fail the build, not silently produce incomplete docs.
- Missing `ToSchema`/`#[utoipa::path]` annotations should be caught during CI validation.

## Implementation Plan

### Step 1: utoipa Setup

Install and configure `utoipa` in the API application. Set up OpenApi derive macro with API title, description, version, and base URL.

### Step 2: Schema Annotation

Derive `ToSchema` on all request/response types across all modules with `#[schema(example = ...)]`, descriptions (doc comments), and nullable annotations.

### Step 3: Handler Annotation

Add `#[utoipa::path(...)]` annotations to all handler functions with `tag`, `params`, `responses` (status codes + body types), and `summary`/`description`.

### Step 4: Reusable Schema Components

Define reusable OpenAPI schema components for: error envelope, pagination envelope, common filter parameters.

### Step 5: utoipa-swagger-ui (Development)

Configure utoipa-swagger-ui at `/api-docs` in development for interactive API exploration.

### Step 6: Spec Export and Publication

Set up spec export as JSON. utoipa-swagger-ui served from the API directly.

### Step 7: CI Validation

Add a CI step that generates the spec and validates completeness (all endpoints documented, no missing schemas).

## Acceptance Criteria

- [ ] `utoipa` integrated and configured
- [ ] All 20+ endpoints documented with decorators
- [ ] Query params documented with types, defaults, and validation rules
- [ ] Response schemas documented for all success responses
- [ ] Error envelopes documented for 400, 404, 500 responses
- [ ] Filter params documented with allowed values per endpoint
- [ ] Pagination format documented (cursor-based envelope)
- [ ] Cache-control behavior noted in endpoint descriptions
- [ ] utoipa-swagger-ui available in development at `/api-docs`
- [ ] OpenAPI spec exportable as JSON
- [ ] utoipa-swagger-ui and spec JSON served directly from the API
- [ ] Spec generated from code (not manually maintained)

## Notes

- This task is best completed after all feature module tasks (0045-0054) are implemented, since it annotates existing controllers and request/response types (ToSchema).
- The spec doubles as a testing contract: frontend developers can mock API responses from the spec.
- utoipa-swagger-ui served from the API directly — no separate infrastructure needed.
