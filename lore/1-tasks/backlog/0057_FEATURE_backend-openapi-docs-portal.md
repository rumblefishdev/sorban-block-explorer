---
id: '0057'
title: 'Backend: OpenAPI spec generation and docs portal'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0042']
tags: [layer-backend, openapi, documentation, swagger]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: OpenAPI spec generation and docs portal

## Summary

Integrate `@nestjs/swagger` to auto-generate an OpenAPI specification from NestJS decorators. The spec documents all 20+ endpoints including query parameters, response schemas, error envelopes, filter parameters, pagination format, and cache-control behavior. Swagger UI is served directly from the API (NestJS `/api-docs` endpoint).

## Status: Backlog

**Current state:** Not started. Depends on task 0023 (NestJS API bootstrap) and all feature module tasks.

## Context

A comprehensive OpenAPI specification serves both as interactive documentation for API consumers and as a machine-readable contract for frontend integration, testing, and third-party tooling. The spec must be auto-generated from the actual NestJS code to stay in sync with the implementation.

### API Specification

**OpenAPI generation:** `@nestjs/swagger` decorators on controllers, DTOs, and response types.

**Publication target:** Swagger UI served directly from the API (NestJS `/api-docs` endpoint). No separate S3 + CloudFront setup.

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
GET /api-docs          -> Swagger UI (interactive docs)
GET /api-docs-json     -> OpenAPI spec as JSON
```

### Example OpenAPI Decorators

```typescript
@ApiOperation({ summary: 'List transactions' })
@ApiQuery({ name: 'limit', type: Number, required: false, description: 'Items per page (default 20, max 100)' })
@ApiQuery({ name: 'cursor', type: String, required: false, description: 'Opaque pagination cursor' })
@ApiQuery({ name: 'filter[source_account]', type: String, required: false })
@ApiResponse({ status: 200, type: TransactionListResponseDto })
@ApiResponse({ status: 400, type: ErrorResponseDto })
```

### Behavioral Requirements

- Spec auto-generated from NestJS decorators (not manually maintained)
- All DTOs annotated with `@ApiProperty()` decorators
- Error envelope documented as reusable schema component
- Pagination envelope documented as reusable schema component
- Swagger UI available in development for interactive testing
- Swagger UI and spec JSON served directly from the API

### Caching

- Published spec is a static file; long-TTL caching on CloudFront is appropriate.
- Swagger UI in development has no caching.

### Error Handling

- Spec generation errors should fail the build, not silently produce incomplete docs.
- Missing decorators should be caught during CI validation.

## Implementation Plan

### Step 1: @nestjs/swagger Setup

Install and configure `@nestjs/swagger` in the API application. Set up Swagger document builder with API title, description, version, and base URL.

### Step 2: DTO Annotation

Annotate all request/response DTOs across all modules with `@ApiProperty()` decorators including types, descriptions, examples, and required/optional flags.

### Step 3: Controller Annotation

Add `@ApiOperation()`, `@ApiQuery()`, `@ApiParam()`, `@ApiResponse()`, and `@ApiTags()` decorators to all controller methods.

### Step 4: Reusable Schema Components

Define reusable OpenAPI schema components for: error envelope, pagination envelope, common filter parameters.

### Step 5: Swagger UI (Development)

Configure Swagger UI at `/api-docs` in development for interactive API exploration.

### Step 6: Spec Export and Publication

Set up spec export as JSON. Swagger UI served from the API directly.

### Step 7: CI Validation

Add a CI step that generates the spec and validates completeness (all endpoints documented, no missing schemas).

## Acceptance Criteria

- [ ] `@nestjs/swagger` integrated and configured
- [ ] All 20+ endpoints documented with decorators
- [ ] Query params documented with types, defaults, and validation rules
- [ ] Response schemas documented for all success responses
- [ ] Error envelopes documented for 400, 404, 500 responses
- [ ] Filter params documented with allowed values per endpoint
- [ ] Pagination format documented (cursor-based envelope)
- [ ] Cache-control behavior noted in endpoint descriptions
- [ ] Swagger UI available in development at `/api-docs`
- [ ] OpenAPI spec exportable as JSON
- [ ] Swagger UI and spec JSON served directly from the API
- [ ] Spec generated from code (not manually maintained)

## Notes

- This task is best completed after all feature module tasks (0045-0054) are implemented, since it annotates existing controllers and DTOs.
- The spec doubles as a testing contract: frontend developers can mock API responses from the spec.
- Swagger UI served from the API directly — no separate infrastructure needed.
