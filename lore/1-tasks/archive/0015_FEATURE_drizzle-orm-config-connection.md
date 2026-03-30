---
id: '0015'
title: 'Drizzle ORM configuration and connection setup'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0004', '0007']
tags: [priority-high, effort-medium, layer-database]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-27
    status: active
    who: stkrolikiewicz
    note: 'Promoted to active'
  - date: 2026-03-27
    status: completed
    who: stkrolikiewicz
    note: >
      Implemented libs/database with Drizzle ORM connection factory.
      11 files created, 4 modified. PR #30.
      Key decisions: pg driver (not postgres.js), framework-agnostic
      factory, env-aware credential resolution, Lambda singleton
      with failed-init retry.
---

# Drizzle ORM configuration and connection setup

## Summary

Set up Drizzle ORM as the database access layer for the Stellar Block Explorer. New `libs/database` library with environment-aware connection factory, Secrets Manager credential resolution, Lambda-optimized connection lifecycle, and Drizzle Kit configuration.

## Status: Completed

## Acceptance Criteria

- [x] Drizzle ORM and Drizzle Kit are installed and configured in the workspace
- [x] Connection factory resolves credentials from Secrets Manager in staging/production
- [x] Connection factory uses local PostgreSQL config in dev environment
- [x] RDS Proxy endpoint is used for all Lambda-to-DB connections in staging and production
- [x] TLS is enforced on production connections
- [x] Module-level connection reuse works correctly in Lambda (warm invocation reuse)
- [ ] apps/api can obtain a Drizzle client through NestJS dependency injection (deferred to 0023 — NestJS not yet installed)
- [x] apps/indexer can obtain a Drizzle client at module scope
- [x] Connection works end-to-end in local dev with a local PostgreSQL instance

## Implementation Notes

**Library:** `libs/database/` — 11 new files

| File                  | Purpose                                                                                    |
| --------------------- | ------------------------------------------------------------------------------------------ |
| `src/connection.ts`   | `getDb()` async singleton, `getPool()`, `closeDb()`                                        |
| `src/credentials.ts`  | `resolveConnectionString()` — env-aware (dev: DATABASE_URL, staging/prod: Secrets Manager) |
| `src/config.ts`       | `DatabaseConfig`, `DatabaseEnvironment`, `resolveEnvironment()`                            |
| `src/schema/index.ts` | Empty barrel for tasks 0016-0020                                                           |
| `src/index.ts`        | Public barrel exports                                                                      |
| `drizzle.config.ts`   | Drizzle Kit CLI config with `path.resolve` for workspace-root execution                    |
| `package.json`        | drizzle-orm 0.45.1, pg 8.20.0, drizzle-kit 0.31.10                                         |

**Root changes:**

- `docker-compose.yml` — PostgreSQL 16 Alpine for local dev
- `eslint.config.mjs` — `scope:database` module boundary rules
- `package.json` — `db:generate`, `db:migrate`, `db:studio` scripts
- `.env.example` — `DATABASE_SECRET_ARN`, `NODE_ENV`
- `tsconfig.json` — database reference added

## Design Decisions

### From Plan

1. **`pg` (node-postgres) driver, not `postgres.js`**: postgres.js uses prepared statements by default, causing RDS Proxy connection pinning and defeating multiplexing.

2. **Framework-agnostic `getDb()` factory**: NestJS DI wrapping deferred to task 0023. Keeps `libs/database` dependency-free from NestJS. One-liner wrap when needed: `{ provide: DRIZZLE, useFactory: () => getDb() }`.

3. **Dynamic import for AWS SDK**: `@aws-sdk/client-secrets-manager` as optional peer dep, loaded only when `DATABASE_SECRET_ARN` is set. Dev never loads it.

4. **`pg.Pool({ max: 1 })`**: One connection per Lambda instance. RDS Proxy handles real pooling across all instances.

5. **Empty schema barrel**: `schema/index.ts` placeholder for tasks 0016-0020 to populate.

### Emerged

6. **Failed-init retry via promise reset**: Original singleton cached the init promise. If `initDb()` failed (e.g., Secrets Manager timeout), the rejected promise was cached forever — Lambda dead until execution environment recycled. Fix: `.catch()` resets `_initPromise` so next invocation retries.

7. **Environment-aware credential resolution**: Original implementation checked `DATABASE_URL` first regardless of environment. PR review caught that this could leak dev credentials into staging/prod. Fix: dev requires `DATABASE_URL`, staging/prod require `DATABASE_SECRET_ARN` exclusively.

8. **`drizzle.config.ts` uses `path.resolve(__dirname)`**: Relative paths in drizzle.config resolve against CWD, not config file location. Since `npm run db:generate` runs from workspace root, `./src/schema/index.ts` would resolve to wrong path. Fixed with `import.meta.url` + `path.resolve`.

9. **Runtime validation of Secrets Manager JSON**: Original used `JSON.parse(...) as { ... }` type assertion. Added runtime type checks on `host`, `port`, `username`, `password`, `dbname` with descriptive error message for malformed secrets.

10. **`@aws-sdk/client-secrets-manager` in devDependencies + peerDependencies**: devDeps for type-checking during build. peerDeps (optional) declares runtime expectation. Lambda Node.js runtime provides SDK v3 — no need to bundle it.

## Issues Encountered

- **Nx socket path too long**: `@nx/vite/plugin` fails with "socket exceeds maximum length" in this repo. Workaround: `NX_DAEMON=false NX_SOCKET_DIR=/tmp/nx-tmp` for all nx commands in hooks.

- **ESM + CJS interop with `pg`**: `pg` ships CJS. Named imports (`import { Pool } from 'pg'`) don't work in ESM. Pattern: `import pg from 'pg'; const { Pool } = pg;`. Works correctly with `module: "nodenext"`.

## Future Work

- NestJS DatabaseModule wrapper (task 0023)

## Notes

- Schema definitions are in tasks 0016-0020. This task is connection plumbing only.
- RDS Proxy is non-negotiable for Lambda — prevents connection exhaustion under burst concurrency.
- `closeDb()` is for tests and graceful shutdown; Lambda environments clean up connections on destroy.
