---
id: '0130'
title: 'DB: ensure monthly partitions before backfill (local + staging)'
type: FEATURE
status: backlog
related_adr: ['0037']
related_tasks: ['0022', '0145', '0167']
tags: [priority-high, effort-small, layer-db, layer-infra, audit-F19]
milestone: 1
links:
  - crates/db-partition-mgmt/src/lib.rs
  - infra/src/lib/stacks/partition-stack.ts
  - lore/3-wiki/backfill-execution-plan.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F19 (MEDIUM).'
  - date: '2026-04-28'
    status: backlog
    who: stkrolikiewicz
    note: >
      Rescoped — original body specced a new migration, but
      `crates/db-partition-mgmt` already implements
      `ensure_time_partitions(SOROBAN_START=2024-02 → today+FUTURE_MONTHS=3)`
      retroactively. Real gap is operational, not schema:
      (a) deploy partition-stack on staging, (b) invoke Lambda once before
      backfill, (c) add a local CLI binary so the same logic runs against
      a docker DB. Triage's "Nov 2023" date was wrong — Soroban activated
      Feb 2024. Type changed BUG → FEATURE because no buggy behavior
      exists; the work is wiring + ops.
---

# DB: ensure monthly partitions before backfill (local + staging)

## Summary

The 7 partitioned parents
(`transactions`, `operations_appearances`, `transaction_participants`,
`soroban_events_appearances`, `soroban_invocations_appearances`,
`nft_ownership`, `liquidity_pool_snapshots`) need monthly children
covering the entire Soroban era before backfill writes the first row —
otherwise every row falls into `_default`, killing partition pruning.

The logic already exists in [`crates/db-partition-mgmt/src/lib.rs`](../../crates/db-partition-mgmt/src/lib.rs)
(Lambda body) — `ensure_time_partitions` covers
`SOROBAN_START=(2024,2) → today+FUTURE_MONTHS=3` retroactively. This task
is the **operational glue** to make that logic run in the two environments
where it currently doesn't:

1. Local docker (no Lambda deployed) — `backfill-bench` only creates
   `_default` today; `backfill-runner` will inherit that gap.
2. Staging RDS — partition-stack must be deployed and Lambda invoked once
   before [the backfill execution plan](../../3-wiki/backfill-execution-plan.md)
   reaches T0.

## Implementation

1. Add a tiny `bin/cli.rs` (or repurpose `main.rs`) in `crates/db-partition-mgmt`
   that reads `DATABASE_URL`, iterates the 7 partitioned tables, and calls
   `ensure_time_partitions(pool, table, today)`. Same code path as the
   Lambda — no duplication.
2. Replace `backfill-bench`'s `ensure_local_default_partitions`
   ([`main.rs:326`](../../crates/backfill-bench/src/main.rs#L326)) call site to
   also invoke the new CLI before indexing, OR document the CLI as a
   manual prerequisite for `backfill-runner`.
3. Deploy `partition-stack` to staging if not already (operations).
4. Force-invoke Lambda once after deploy so the backfill window is covered
   (operations).

## Acceptance Criteria

- [ ] CLI binary in `crates/db-partition-mgmt` runs locally and creates
      missing children for all 7 tables
- [ ] After CLI run, `pg_inherits` shows ≈ 7 × (today − 2024-02 in months + 3)
      children + 7 `_default` (no fewer)
- [ ] backfill-runner README points at the CLI as a prerequisite
- [ ] partition-stack deployed on staging
- [ ] Lambda invocation logged as covering full Soroban era

## Out of scope

- `_default` retention — handled by [partition-pruning runbook](../../3-wiki/partition-pruning-runbook.md)
- Future-month rollover — Lambda EventBridge cron already does this (task 0022)
