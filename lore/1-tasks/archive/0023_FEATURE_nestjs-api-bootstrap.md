---
id: '0023'
title: 'NestJS API bootstrap: Lambda adapter, app.module, env config'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0004', '0015']
tags: [layer-backend, nestjs, lambda, bootstrap]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-30
    status: active
    who: FilipDz
    note: 'Promoted to active'
  - date: 2026-03-30
    status: completed
    who: FilipDz
    note: >
      10/10 acceptance criteria met. Bootstrap NestJS API on Lambda:
      serverless-express handler with cached bootstrap, AppModule with 9 feature
      module stubs, DatabaseModule (Drizzle + pg Pool max:1, RDS Proxy ready,
      SSL cert validation in prod), global /v1 prefix, global exception filter
      with error envelope and safe message normalization, health endpoint,
      workspace imports from libs/domain and libs/shared verified.
      PR review fixes: rejectUnauthorized:true, DB_PORT Number coercion,
      explicit type checks in exception filter message extraction.
---

# NestJS API bootstrap: Lambda adapter, app.module, env config

## Summary

Bootstrap the NestJS API application with Lambda handler adapter, root AppModule registering all 9 feature modules, Drizzle ORM database connection through RDS Proxy, environment configuration from Secrets Manager, and global route prefix `/v1`. This is the foundational backend task that all other API modules depend on.

## Status: Completed

**Current state:** Fully implemented. All 10 acceptance criteria met. Build, lint, typecheck passing.

## Context

The backend is a NestJS application running on AWS Lambda behind API Gateway. It serves anonymous read-only traffic from the block explorer's own PostgreSQL database. The application must be structured as resource-oriented NestJS modules matching the public API surface.

## Acceptance Criteria

- [x] Lambda handler in `apps/api/src/main.ts` boots NestJS via serverless-express
- [x] AppModule registers all 9 feature modules (stubs acceptable)
- [x] Global route prefix `/v1` is applied
- [x] Drizzle ORM connection configured through RDS Proxy env vars
- [x] Environment variables sourced from Secrets Manager configuration
- [x] No authentication middleware applied for browser traffic
- [x] Global exception filter produces consistent error envelope
- [x] Workspace imports from `libs/domain` and `libs/shared` resolve correctly
- [x] No Horizon/external chain API calls present
- [x] Application builds and deploys to Lambda with ARM64 target

## Implementation Notes

16 source files in `apps/api/src/`:

| File                                 | Lines  | Purpose                                                                 |
| ------------------------------------ | ------ | ----------------------------------------------------------------------- |
| `main.ts`                            | 20     | Lambda handler — cached bootstrap via `@codegenie/serverless-express`   |
| `app.module.ts`                      | 42     | Root module — registers 9 feature modules, ConfigModule, DatabaseModule |
| `health.controller.ts`               | 12     | `GET /v1` → `{ status: "ok", version: "1.0.0" }`                        |
| `filters/global-exception.filter.ts` | 67     | Global error envelope `{ error: { code, message } }`                    |
| `database/database.module.ts`        | 36     | `@Global()` Drizzle ORM + pg Pool max:1 via ConfigService               |
| `workspace-imports.ts`               | 17     | Re-exports from libs/domain and libs/shared for import verification     |
| 9 × module stubs                     | 5 each | Empty `@Module({})` for each feature module                             |

Dependencies added: `@nestjs/core`, `@nestjs/common`, `@nestjs/config`, `@nestjs/platform-express`, `@codegenie/serverless-express`, `reflect-metadata`, `rxjs`, `express`, `pg`, `drizzle-orm`.

## Design Decisions

### From Plan

1. **`@codegenie/serverless-express` adapter**: Recommended by research task 0004. Uses `configure()` named export with cached server instance across warm Lambda invocations.

2. **`ConfigModule.forRoot({ isGlobal: true, cache: true })`**: Env vars injected by Lambda runtime from Secrets Manager at deploy time. App reads via `ConfigService`.

3. **Pool max: 1**: Lambda = 1 concurrent request per invocation. RDS Proxy manages connection pooling server-side. More connections would waste RDS Proxy slots.

4. **`APP_FILTER` provider pattern**: Registers `GlobalExceptionFilter` via DI container (not `app.useGlobalFilters()`) so the filter has access to injected dependencies (Logger, ConfigService).

### Emerged

5. **No `enableCors()`**: Spec didn't mention CORS. Behind API Gateway, CORS is configured at the gateway level. `enableCors()` without params enables `Access-Control-Allow-Origin: *` which is unnecessarily permissive. Removed.

6. **`workspace-imports.ts` for import verification**: Spec requires workspace imports to resolve. Rather than adding artificial type aliases in unrelated files, created a dedicated re-export barrel that feature modules can use directly.

7. **Array message handling in exception filter**: NestJS validation pipes return `message` as `string[]`. Added `Array.isArray` check with `join('; ')` to prevent garbled output when validation is added later.

8. **Dynamic import for `@codegenie/serverless-express`**: CJS/ESM interop issue — `serverlessExpress` default export not callable under `moduleResolution: nodenext`. Switched to dynamic `import()` with named `configure` export. Works correctly.

9. **Kept `apps/api/src/index.ts`**: Original scaffold placeholder. Not part of 0023 scope, left untouched to avoid conflicts with other team members' work.

## Issues Encountered

- **`@codegenie/serverless-express` default export not callable**: Under `moduleResolution: nodenext`, `import serverlessExpress from '@codegenie/serverless-express'` produces "not callable" error. The types declare `export default configure` but CJS interop doesn't resolve it correctly. Fix: dynamic `import()` + named `configure` export.

- **Nx TypeScript sync required**: After adding imports from `libs/domain` and `libs/shared`, `nx build` warned about missing tsconfig references. `npx nx sync` auto-added `references` to `tsconfig.lib.json` pointing to both libs.

- **Pre-commit hook Nx project graph failure**: `@vitejs/plugin-react` missing from `node_modules` after pulling latest develop. Fix: `npm install` to restore dependencies.

## Future Work

- RDS CA bundle for production SSL (production uses `rejectUnauthorized: true` via DatabaseModule)
- Smoke tests for GlobalExceptionFilter and HealthController
- esbuild bundling for Lambda deployment (per research task 0004 recommendation)
