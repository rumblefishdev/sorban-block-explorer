---
id: '0130'
title: 'DB: ensure monthly partitions before backfill (local + staging)'
type: FEATURE
status: completed
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
  - date: '2026-04-28'
    status: active
    who: stkrolikiewicz
    note: 'Promoted to active via /promote-task — single blocker for backfill T0 per `lore/3-wiki/backfill-execution-plan.md`.'
  - date: '2026-04-28'
    status: completed
    who: stkrolikiewicz
    note: >
      Code shipped in PR #135. New `bin/cli` in `crates/db-partition-mgmt`
      reuses the Lambda's `ensure_default_partition` + `ensure_time_partitions`
      primitives against `DATABASE_URL`. backfill-bench delegates its
      `_default` setup to the shared lib (no inline DDL). backfill-runner
      README + tech-design overview + backfill-execution-plan wiki updated
      to point operators at the CLI. Verified locally: 184 monthly children
      + 7 `_default` provisioned; idempotent re-run returns 0 created;
      manually-detached `_default` is reattached on the next CLI run.
      Staging deploy + first Lambda invoke deferred to the staging-cutover
      operations track (out of scope of this PR by design).
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

- [x] CLI binary in `crates/db-partition-mgmt` runs locally and creates
      missing children for all 7 tables
- [x] After CLI run, `pg_inherits` shows ≈ 7 × (today − 2024-02 in months + 3)
      children + 7 `_default` (no fewer) — verified empirically:
      7 × 31 (30 monthly + 1 default) for today = 2026-04-28
- [x] backfill-runner README points at the CLI as a prerequisite
- [ ] partition-stack deployed on staging — deferred to staging-cutover
      operations track; no code work outstanding
- [ ] Lambda invocation logged as covering full Soroban era — same as above

## Implementation Notes

- New `src/bin/cli.rs` (~80 LOC) wired through `clap` against
  `DATABASE_URL`; calls `ensure_all_partitions(pool, today)`.
- `ensure_default_partition` extracted into the lib so the Lambda, the
  CLI, and `backfill-bench` all share one implementation. The Lambda
  handler now calls it before `ensure_time_partitions` per table so CLI
  and Lambda provision identically.
- `ensure_default_partition` rewritten during PR review to pre-check
  `pg_inherits` + `to_regclass` and dispatch to one of three explicit
  branches (already-attached / detached / missing) instead of relying on
  the SQLSTATE 42P07 sentinel — the original `CREATE TABLE IF NOT EXISTS`
  silently swallowed the detached case and would have left a manually
  detached `_default` invisible to inserts.

## Design Decisions

### From Plan

1. **Reuse the Lambda code path** — the original task body called for it,
   delivered via `bin/cli.rs` + extracted `ensure_default_partition`
   helper.
2. **`backfill-bench` keeps its `_default`-only shortcut** rather than
   forcing every smoke run to provision 184 monthly children. The bench
   stays cheap; the CLI is a one-liner away when full coverage matters.

### Emerged

3. **Lambda also calls `ensure_default_partition`** — surfaced by the
   PR-#135 review. Without it, the Lambda's per-table loop would have
   diverged from the CLI on the `_default` step, even though the
   docstring claimed parity. Added a single call inside the existing
   loop; no metric or behavioral regression.
4. **Three-state `pg_inherits` pre-check instead of SQLSTATE 42P07
   sentinel.** The first version copied the `ensure_time_partitions`
   pattern (`CREATE TABLE IF NOT EXISTS` → catch 42P07 → `ATTACH`), but
   the `IF NOT EXISTS` clause suppresses 42P07 entirely, leaving the
   reattach branch unreachable. Rewrote during PR review.
5. **Per-table CloudWatch dimension preserved.** The Lambda could have
   collapsed to one `ensure_all_partitions` call, but that would have
   lost the per-table `FuturePartitionCount` metric the existing
   monitoring relies on. Kept the loop; absorbed the new
   `ensure_default_partition` into it.

## Issues Encountered

- **Smoke test against my pre-existing 110k-row `_default`** (left over
  from prior 300-ledger backfill runs) blocked the first CLI invocation
  with PG error 23514 ("would be violated by some row"). The CLI now
  surfaces a remediation hint pointing operators at `TRUNCATE` for
  scratch DBs and at the partition-pruning runbook for staging. Not a
  regression — `_default` rows in a month we're trying to add really do
  block the CREATE; the hint just makes the recovery action obvious.

## Out of scope

- `_default` retention — handled by [partition-pruning runbook](../../3-wiki/partition-pruning-runbook.md)
- Future-month rollover — Lambda EventBridge cron already does this (task 0022)
