---
id: '0021'
title: 'Database migration framework'
type: FEATURE
status: completed
related_adr: ['0005']
related_tasks: ['0015', '0031', '0092']
tags: [priority-high, effort-medium, layer-database]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Rewritten per ADR 0005 + research 0092: Drizzle Kit → sqlx migration framework'
  - date: 2026-04-01
    status: active
    who: stkrolikiewicz
    note: 'Activated for implementation'
  - date: 2026-04-02
    status: completed
    who: stkrolikiewicz
    note: >
      All 7 implementation steps complete. sqlx-cli + migrations/ dir with 4 SQL
      migrations, sqlx::migrate!() in crates/db, db-migrate Lambda binary with
      CDK custom resource (MigrationStack), npm scripts for local dev workflow,
      MIGRATIONS.md documenting all commands. CDK dependency chain enforces
      RdsStack → MigrationStack → ComputeStack ordering.
---

# Database migration framework

## Summary

sqlx migration framework: sqlx-cli, sqlx::migrate!() embedding, CI sqlx migrate run, SQLX_OFFLINE mode. Migrations must complete successfully before new Lambda code (API and Indexer) is deployed -- migration failure blocks deployment.

## Status: Completed

## Context

The block explorer uses sqlx for database access and sqlx-cli for schema management and migration generation (per ADR 0005). The migration framework must work across three environments with different connection characteristics and must integrate into the CDK deployment pipeline as a hard prerequisite for application code deployment.

### Environment Handling

| Environment | Connection                | Migration Method                                        |
| ----------- | ------------------------- | ------------------------------------------------------- |
| dev         | Local PostgreSQL (direct) | `sqlx migrate run` via sqlx-cli against local DB        |
| staging     | RDS through RDS Proxy     | `sqlx migrate run` or CDK custom resource through proxy |
| production  | RDS through RDS Proxy     | `sqlx migrate run` or CDK custom resource through proxy |

- In dev, migrations run directly via sqlx-cli commands (e.g., `sqlx migrate run`).
- In staging and production, migrations run through a dedicated migration step that connects through RDS Proxy, executed as part of the CDK deployment pipeline.

### CDK Integration Requirements

- Migrations MUST complete before deploying new Lambda code for both apps/api and apps/indexer.
- Migration failure MUST block the deployment -- no new application code is rolled out if the schema is not in the expected state.
- This can be implemented as a CDK custom resource, a pre-deployment Lambda, or a CodeBuild step within the CDK pipeline. The mechanism must guarantee ordering.

### Schema Evolution Rules

From the architecture documentation, schema changes must follow these rules:

- Add new tables or columns only when tied to a documented explorer or ingestion need.
- Never replace explicit relational structure with oversized generic JSON blobs.
- Keep public lookup keys stable where routes or API contracts depend on them.
- Update the general architecture overview first if the conceptual schema changes materially.

### Migration Versioning

- Migration files are plain SQL, managed by sqlx-cli, and committed to source control in `migrations/`.
- Each migration is a versioned, ordered SQL file that represents an incremental schema change.
- `sqlx::migrate!()` embeds migrations at compile time for the Rust binary.
- CI validates migrations apply cleanly. `SQLX_OFFLINE=true` mode is used for CI builds without a live database.

## Implementation Plan

### Step 1: sqlx-cli setup and migration directory

Install `sqlx-cli` (`cargo install sqlx-cli --no-default-features --features postgres`). Create `migrations/` directory at the workspace level for plain SQL migration files. Configure `DATABASE_URL` resolution per environment.

### Step 2: Migration directory structure

Establish the migration directory within the workspace. Migrations should live in a location that is:

- Version-controlled alongside the Rust crate source.
- Accessible to both the local CLI workflow and the CDK deployment pipeline.
- Named with sqlx-cli convention: `{timestamp}_{description}.sql`.

### Step 3: Local dev migration workflow

Set up the local development workflow:

- `sqlx migrate add <name>` to create new migration files.
- `sqlx migrate run` to apply migrations to local PostgreSQL.
- `sqlx migrate revert` for rolling back the most recent migration during development.

### Step 4: sqlx::migrate!() embedding

Embed migrations at compile time using `sqlx::migrate!()` in the Rust binary. This ensures the deployed binary includes all migrations and can run them on startup or via a dedicated migration entrypoint.

### Step 5: SQLX_OFFLINE mode for CI

Configure `SQLX_OFFLINE=true` for CI builds. Generate and commit `sqlx-data.json` (or `.sqlx/` directory) so that `cargo build` succeeds without a live database. Add a CI step to verify the offline data is up-to-date.

### Step 6: CDK migration integration

Implement the CDK deployment integration:

- Create a migration execution mechanism (custom resource Lambda or CodeBuild step) that runs `sqlx migrate run`.
- Wire it into the CDK deployment pipeline so it runs BEFORE Lambda function updates.
- Ensure the migration step connects through RDS Proxy for staging/production.
- Implement failure handling: if migration fails, the deployment is aborted.

### Step 7: Rollback strategy

Document and implement the rollback approach:

- sqlx supports reversible migrations via `sqlx migrate revert`. Write down-migration SQL when needed.
- For non-destructive changes (adding columns/tables), rollback may not be necessary.
- For destructive changes, a manual rollback migration must be prepared and tested before deployment.

## Acceptance Criteria

- [x] `migrations/` directory contains plain SQL migration files managed by sqlx-cli
- [x] Migration files are committed to source control
- [x] Local dev workflow works: `sqlx migrate add`, `sqlx migrate run`, and `sqlx migrate revert` function against local PostgreSQL
- [x] `sqlx::migrate!()` embeds migrations at compile time in the Rust binary
- [x] `SQLX_OFFLINE=true` mode works for CI builds without a live database
- [x] CDK pipeline runs `sqlx migrate run` before deploying new Lambda code
- [x] Migration failure blocks deployment (no partial rollout of code without schema)
- [x] Staging and production migrations connect through RDS Proxy
- [ ] Migration files apply cleanly to a fresh PostgreSQL instance in CI (deferred to 0039 — CI/CD pipeline not yet implemented)

## Implementation Notes

### Files created/modified

| File                                              | Purpose                                                                                 |
| ------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `crates/db/migrations/0001-0004_*.sql`            | 4 initial schema migrations (ledgers, transactions, operations, soroban)                |
| `crates/db/src/migrate.rs`                        | `run_migrations()` using `sqlx::migrate!()` compile-time embedding                      |
| `crates/db/src/lib.rs`                            | Module exports: migrate, pool, secrets                                                  |
| `crates/db/src/pool.rs`                           | PgPool configuration                                                                    |
| `crates/db/src/secrets.rs`                        | AWS Secrets Manager credential resolution                                               |
| `crates/db-migrate/src/main.rs`                   | Lambda binary — CloudFormation custom resource handler                                  |
| `infra/aws-cdk/src/lib/stacks/migration-stack.ts` | CDK stack: RustFunction + Provider + CustomResource                                     |
| `infra/aws-cdk/src/lib/app.ts`                    | Dependency chain: `migration.addDependency(rds)` + `compute.addDependency(migration)`   |
| `crates/db/MIGRATIONS.md`                         | Developer docs: all commands, conventions, rollback                                     |
| `package.json`                                    | npm scripts: `db:migrate`, `db:revert`, `db:add`, `db:status`, `db:prepare`, `db:reset` |

### CDK deployment ordering

```
NetworkStack → RdsStack → MigrationStack → ComputeStack
```

MigrationStack uses a CDK `CustomResource` with `Provider` wrapping a Rust Lambda. On every deploy, `Date.now()` forces re-invocation. sqlx migrations are idempotent — already-applied ones are skipped.

## Design Decisions

### From Plan

1. **sqlx-cli for migration management**: Per ADR 0005, using plain SQL files managed by sqlx-cli.
2. **`sqlx::migrate!()` compile-time embedding**: Migrations baked into the binary, no runtime file access needed.
3. **CDK custom resource for deployment migration**: CloudFormation custom resource guarantees ordering — migration failure rolls back the stack.

### Emerged

4. **Separate `db-migrate` binary crate**: Lambda handler is a standalone crate rather than a feature flag on `crates/db`. Cleaner separation — the migration Lambda has its own `main.rs` and Lambda dependencies.
5. **`Date.now()` for forced re-invocation**: CloudFormation custom resources only trigger on property changes. Using timestamp ensures migrations run on every deployment even if no properties changed.
6. **Initial migrations are irreversible (0001-0004)**: Foundational schema — reverting would destroy all data. New migrations use `-r` flag for paired up/down files.
7. **npm scripts as dev DX layer**: `npm run db:migrate` etc. wraps sqlx-cli with hardcoded local DATABASE_URL — avoids .env files for local dev.

## Notes

- CI validation of migrations (apply cleanly to fresh PostgreSQL) is deferred to task 0039 (CI/CD pipeline).
- Migration ordering is critical: all schema tasks (0016-0020) produce SQL DDL consumed by this framework.
- `.sqlx/` directory for SQLX_OFFLINE is not yet committed — will be generated once compile-time checked queries are added.
