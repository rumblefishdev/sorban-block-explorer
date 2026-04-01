---
id: '0021'
title: 'Database migration framework'
type: FEATURE
status: backlog
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
---

# Database migration framework

## Summary

sqlx migration framework: sqlx-cli, sqlx::migrate!() embedding, CI sqlx migrate run, SQLX_OFFLINE mode. Migrations must complete successfully before new Lambda code (API and Indexer) is deployed -- migration failure blocks deployment.

## Status: Backlog

**Current state:** Not started.

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

- [ ] `migrations/` directory contains plain SQL migration files managed by sqlx-cli
- [ ] Migration files are committed to source control
- [ ] Local dev workflow works: `sqlx migrate add`, `sqlx migrate run`, and `sqlx migrate revert` function against local PostgreSQL
- [ ] `sqlx::migrate!()` embeds migrations at compile time in the Rust binary
- [ ] `SQLX_OFFLINE=true` mode works for CI builds without a live database
- [ ] CDK pipeline runs `sqlx migrate run` before deploying new Lambda code
- [ ] Migration failure blocks deployment (no partial rollout of code without schema)
- [ ] Staging and production migrations connect through RDS Proxy
- [ ] Migration files apply cleanly to a fresh PostgreSQL instance in CI

## Notes

- This task depends on the sqlx database connection configuration being in place.
- The specific CDK integration mechanism (custom resource vs. CodeBuild step) should be decided during implementation based on CDK best practices and the existing pipeline structure.
- Migration ordering is critical: all schema tasks (0016-0020) produce SQL DDL, but the migration framework must be ready to apply their migrations.
- Task 0031 (referenced in related_tasks) covers broader CDK infrastructure. The migration integration should align with that infrastructure design.
