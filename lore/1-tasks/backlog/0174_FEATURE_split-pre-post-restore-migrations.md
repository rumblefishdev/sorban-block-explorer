---
id: '0174'
title: "DB migrations: split pre-restore vs post-restore directories so the Lambda doesn't apply heavy indexes before pg_restore"
type: FEATURE
status: backlog
related_adr: ['0032', '0039']
related_tasks: ['0132']
tags: [priority-medium, effort-small, layer-db, layer-infra, backfill, staging]
milestone: 1
links:
  - crates/db-migrate/src/main.rs
  - crates/db/migrations/20260428000100_add_endpoint_query_indexes.up.sql
  - lore/3-wiki/backfill-execution-plan.md
history:
  - date: '2026-04-28'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from PR #137 (task 0132) review thread. The standard
      migration Lambda (`crates/db-migrate`) calls
      `sqlx::migrate!("./migrations").run(pool)` and applies every
      pending migration at infra deploy time. For the staging-cutover
      flow described in `backfill-execution-plan.md`, that's wrong:
      the 0132 indexes (and any future heavy-scan migration) need to
      land **after** `pg_restore` of the per-month bundles, not
      before — otherwise every `COPY` row pays per-index-update cost
      and the restore stretches by hours.
---

# DB migrations: split pre-restore vs post-restore directories

## Summary

Today the migration Lambda blindly applies every migration in
`crates/db/migrations/` at infra deploy. That's correct for greenfield
deploys, but wrong for the staging-cutover flow that loads a backfilled
DB via `pg_restore`: heavy-scan migrations (CREATE INDEX on tables
about to be filled by COPY) belong **after** the restore, not before.
Split the migration set into two directories so operators can defer
the post-restore set explicitly without env-var gymnastics or runbook
reliance on "remember to drop indexes before restore".

## Context

PR #137 ([task 0132](../archive/0132_FEATURE_missing-db-indexes.md))
adds five read-path indexes via migration `20260428000100`. The
migration is correct, but its placement inside the standard
`crates/db/migrations/` directory means the migration Lambda will
apply it on every infra deploy — including the deploy that immediately
precedes a `pg_restore`.

Concrete impact at staging cutover (T5–T6 in
[`backfill-execution-plan.md`](../../3-wiki/backfill-execution-plan.md)):

- **With current single-directory setup**:

  1. CDK deploy → Lambda runs all migrations including 0132
  2. `pg_restore` Bundle M (per-month) → COPY rows updates 5 indexes
     per row → ~hours of B-tree maintenance instead of a sequential
     batch build at the end
  3. Operator must manually `DROP INDEX` before pg_restore and
     re-`CREATE` after, defeating the migration system's invariant

- **Desired**:
  1. CDK deploy → Lambda runs **pre-restore** migrations only (schema
     - base indexes, no heavy-scan)
  2. `pg_restore` Bundle M → fast COPY, no extra index maintenance
  3. T6 manual step → run **post-restore** migrations → indexes built
     once on populated tables in a single sequential pass

## Implementation Plan

### Step 1: Directory split

```
crates/db/migrations/                  # pre-restore (schema, base indexes)
crates/db/migrations-post-restore/     # heavy-scan migrations
```

Move migration `20260428000100_add_endpoint_query_indexes.{up,down}.sql`
to the new `migrations-post-restore/` directory. Same timestamp, no
sqlx checksum disturbance because the `_sqlx_migrations` table is
per-database and the standard Lambda no longer sees this migration.

### Step 2: `db::migrate` library refactor

`crates/db/src/migrate.rs` today has:

```rust
sqlx::migrate!("./migrations").run(pool).await
```

Split into two functions:

```rust
pub async fn run_pre_restore(pool: &PgPool) -> Result<...> {
    sqlx::migrate!("./migrations").run(pool).await
}

pub async fn run_post_restore(pool: &PgPool) -> Result<...> {
    sqlx::migrate!("./migrations-post-restore").run(pool).await
}
```

`sqlx::migrate!` is a macro — both directories must exist at compile
time. Empty directory with just `.gitkeep` is fine for an early state.

### Step 3: Lambda binary refactor

`crates/db-migrate/src/main.rs` accepts a `MIGRATION_PHASE` env var:

```rust
match std::env::var("MIGRATION_PHASE").as_deref() {
    Ok("pre-restore") | Err(_) => db::migrate::run_pre_restore(&pool).await?,
    Ok("post-restore")        => db::migrate::run_post_restore(&pool).await?,
    Ok(other) => return Err(format!("unknown MIGRATION_PHASE: {other}").into()),
}
```

Default `pre-restore` keeps every existing CFN deploy working. Staging
cutover sets `MIGRATION_PHASE=post-restore` for the T6 manual invoke.

### Step 4: CDK wiring

The migration Lambda is invoked from a CFN custom resource. Two
options:

- (a) Two Lambdas (one per phase), invoked at different stack stages
- (b) One Lambda with the phase passed via custom-resource property

Pick (b) — simpler, single binary, single IAM role. CDK changes:
the custom-resource invocation in `infra/src/lib/stacks/...` passes
`MigrationPhase` parameter, the Lambda reads it from the CFN event
payload, sets the env var, and runs the matching `migrate!` macro
output.

### Step 5: T6 wiring in `backfill-execution-plan.md`

Replace the current ambiguous T6 step:

```
T6  ANALYZE staging.*
```

with:

```
T6a  Manual invoke db-migrate Lambda with MigrationPhase=post-restore
     (or `cargo run -p db-migrate -- --phase post-restore` against
     a tunneled DSN)
T6b  ANALYZE staging.*
```

### Step 6: Add a third stub migration to `migrations-post-restore/`

A no-op `0001_init.up.sql` (`SELECT 1;`) so the macro has at least one
migration to embed; also documents the convention for the next
heavy-scan candidate.

## Acceptance Criteria

- [ ] `crates/db/migrations-post-restore/` directory exists with at
      least one migration (the moved 0132 migration; or a stub if
      0132 has already been re-applied via the standard set on
      staging — check before starting)
- [ ] `crates/db/src/migrate.rs` exposes `run_pre_restore` and
      `run_post_restore` functions; old `run_migrations` either
      becomes an alias for `run_pre_restore` or is removed (callers
      updated)
- [ ] `crates/db-migrate/src/main.rs` switches on `MIGRATION_PHASE`
      env var (default = `pre-restore`)
- [ ] CDK custom-resource invocation passes `MigrationPhase`
- [ ] `backfill-execution-plan.md` T6 step updated to call the
      post-restore phase explicitly
- [ ] Smoke: standalone test DB applies pre-restore migrations
      cleanly, then post-restore migrations cleanly, then a
      simulated pg_restore between them does not double-apply
- [ ] **Docs updated** —
      `docs/architecture/technical-design-general-overview.md` and
      `docs/architecture/infrastructure/infrastructure-overview.md`
      reflect the two-phase migration approach (per ADR 0032)
- [ ] Optional: ADR 0040 documenting the split-directory pattern as
      the canonical answer to "this migration is heavy and should
      only run on a populated DB"

## Notes

- **Why not env-var gating inside a single directory?** sqlx's
  `migrate!` macro embeds migrations at compile time and applies them
  unconditionally; filtering by version requires either a custom
  `Migrate` impl or a wrapper that reads `_sqlx_migrations` and
  re-runs selectively. Both add code surface that's harder to reason
  about than two directories.
- **Why not "drop indexes before restore"?** Possible but
  operator-dependent and forgettable. Two directories make the
  expectation visible in the file system layout.
- **Out of scope:** moving any _existing_ applied migration. 0132 is
  the first candidate; future heavy-scan migrations follow the same
  rule. Pre-existing indexes (created in migrations 0001–0007) stay
  in the pre-restore set.
