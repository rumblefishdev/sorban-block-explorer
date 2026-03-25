---
id: '0023'
title: 'NestJS API bootstrap: Lambda adapter, app.module, env config'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0004', '0015']
tags: [layer-backend, nestjs, lambda, bootstrap]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# NestJS API bootstrap: Lambda adapter, app.module, env config

## Summary

Bootstrap the NestJS API application with Lambda handler adapter, root AppModule registering all 9 feature modules, Drizzle ORM database connection through RDS Proxy, environment configuration from Secrets Manager, and global route prefix `/v1`. This is the foundational backend task that all other API modules depend on.

## Status: Backlog

**Current state:** Not started. Depends on research task 0004 (NestJS Lambda adapter) and task 0015 (Drizzle ORM config/connection).

## Context

The backend is a NestJS application running on AWS Lambda behind API Gateway. It serves anonymous read-only traffic from the block explorer's own PostgreSQL database. The application must be structured as resource-oriented NestJS modules matching the public API surface.

### API Specification

**Base URL:** `https://api.soroban-explorer.com/v1`

**Global route prefix:** `/v1`

**Runtime:** AWS Lambda with ARM64/Graviton2, using serverless-express adapter.

**Files:**

- `apps/api/src/main.ts` -- Lambda handler adapter (serverless-express)
- `apps/api/src/app.module.ts` -- Root module

### Root Module Registration

The AppModule must register all 9 feature modules:

1. NetworkModule
2. TransactionsModule
3. LedgersModule
4. AccountsModule
5. TokensModule
6. ContractsModule
7. NFTsModule
8. LiquidityPoolsModule
9. SearchModule

### Database Connection

- Drizzle ORM connection through RDS Proxy
- Environment variables sourced from AWS Secrets Manager
- Connection pooling managed by RDS Proxy (not application-level)

### Authentication and Abuse Controls

- Anonymous read-only traffic; NO authentication middleware for browsers
- API keys reserved for trusted non-browser consumers only (optional API Gateway usage plans)
- Abuse controls enforced at API Gateway + AWS WAF layer, NOT in NestJS

### Architectural Prohibitions

1. No live chain indexing in the API layer
2. No Horizon or external chain API calls
3. No third-party explorer database dependencies
4. No shifting protocol interpretation to frontend
5. All data served from the block explorer's own RDS

### Workspace Imports

- `libs/domain` for shared explorer-domain types
- `libs/shared` for generic cross-cutting helpers

### Response Shape (root health/info)

```json
{
  "status": "ok",
  "version": "1.0.0"
}
```

### Error Envelope (global)

```json
{
  "error": {
    "code": "string",
    "message": "string"
  }
}
```

### Caching

- No caching at this bootstrap level; caching is configured per-module and at API Gateway layer.

### Error Handling

- Global exception filter mapping errors to consistent error envelope
- 400 for validation failures
- 404 for missing resources
- 500 for internal errors

## Implementation Plan

### Step 1: Lambda Handler Adapter

Configure `apps/api/src/main.ts` with serverless-express adapter. Set up the Lambda handler function that bootstraps NestJS and delegates to the express adapter. Target ARM64/Graviton2 runtime.

### Step 2: AppModule Root Registration

Create `apps/api/src/app.module.ts` registering all 9 feature modules (initially as empty placeholder modules). Import Drizzle ORM module with RDS Proxy connection configuration.

### Step 3: Environment Configuration

Set up environment variable loading from Secrets Manager. Configure database connection string, region, and any feature flags. Use NestJS ConfigModule or equivalent.

### Step 4: Global Route Prefix

Apply global prefix `/v1` to all routes so all endpoints are served under `https://api.soroban-explorer.com/v1/*`.

### Step 5: Global Exception Filter

Implement a global exception filter that maps all errors to the standard error envelope format: `{ error: { code, message } }`.

### Step 6: Workspace Import Verification

Verify that `libs/domain` and `libs/shared` are importable from the `apps/api` application within the Nx workspace.

## Acceptance Criteria

- [ ] Lambda handler in `apps/api/src/main.ts` boots NestJS via serverless-express
- [ ] AppModule registers all 9 feature modules (stubs acceptable)
- [ ] Global route prefix `/v1` is applied
- [ ] Drizzle ORM connection configured through RDS Proxy env vars
- [ ] Environment variables sourced from Secrets Manager configuration
- [ ] No authentication middleware applied for browser traffic
- [ ] Global exception filter produces consistent error envelope
- [ ] Workspace imports from `libs/domain` and `libs/shared` resolve correctly
- [ ] No Horizon/external chain API calls present
- [ ] Application builds and deploys to Lambda with ARM64 target

## Notes

- This task is the foundation for all backend API tasks (0024-0038).
- The 9 feature modules can initially be empty NestJS modules with no endpoints; each will be fleshed out by its own task.
- API Gateway and WAF configuration are infrastructure concerns handled separately.
