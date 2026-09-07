---
id: '0361'
title: 'Port audit-harness to ClickHouse (data-correctness safety net)'
type: FEATURE
status: superseded
related_adr: []
related_tasks: ['0244', '0175']
tags: ['clickhouse', 'audit', 'data-correctness', 'tooling']
links: []
history:
  - date: '2026-08-30'
    status: superseded
    who: karolkow
    by: ['0524']
    note: >
      Superseded by the 0524 verification-harness umbrella (one-flow
      decision): porting the sqlx SQL is a rewrite anyway, and a second
      tool beside snapshot-seed is exactly what the consolidation forbids.
      The crate's invariant catalog is transcribed into 0524; the crate
      itself is deleted from the tree.
  - date: 2026-07-06
    status: backlog
    who: karolkow
    note: 'Spawned from 0244 (PG removal).'
  - date: 2026-07-07
    status: backlog
    who: karolkow
    note: >
      Re-scoped rebuild→PORT. audit-harness was first deleted then reverted in
      0244 once the code was re-read: the 3 diff binaries (~2760 lines) are
      store-agnostic, only ~32 lines touch sqlx. So the CH work is a port (swap
      the DB-read layer, re-express the SQL invariants), not a from-scratch
      rewrite. Crate stays in-tree; 0244 carves out the sqlx workspace-dep drop
      until this lands.
---

# Port audit-harness to ClickHouse (data-correctness safety net)

## Summary

`crates/audit-harness` is the automated data-correctness safety net — per-table
SQL invariants across all rows plus DB↔Horizon and DB↔archive-XDR diffs, the
automated complement to the manual PR audits (0167/0172/0173). It is
`sqlx`/PostgreSQL-bound and became non-functional at the ClickHouse cutover.
Task 0244 kept the crate (reverted an initial delete) and defers the CH work
here. This task **ports** it to ClickHouse — reusing the tested store-agnostic
logic, swapping only the DB-read layer — rather than rewriting from scratch.

## Status: Backlog

**Current state:** not started. The crate is back in the workspace (0244 commit
`4bd834b8`), still PG-bound, so it does not run against the live CH store. 0244's
"drop the `sqlx` workspace dep" endpoint is carved out until this port lands —
audit-harness is the sole remaining `sqlx` user.

## Context — why PORT, not rewrite

Measured on the reverted code:

| Binary                  | Lines | `sqlx` lines | Store-agnostic logic (ports as-is)             |
| ----------------------- | ----: | -----------: | ---------------------------------------------- |
| `horizon-diff`          |  1272 |           15 | Horizon client + field diff                    |
| `archive-diff`          |   625 |            6 | archive `.xdr.zst` re-parse + CAP-0038 pool-id |
| `operations-order-diff` |   863 |           11 | apply-order preservation check                 |

**~2760 binary lines, ~32 touch `sqlx`.** The valuable parts — Horizon fetch,
archive XDR re-parse, CAP-0038 pool-id reconstruction — are store-agnostic and
port unchanged; only the thin DB-read layer swaps `sqlx` → `clickhouse::Client`
(mTLS via `db_clickhouse::mtls` for prod, plain client for local). Rewriting
those from scratch would discard tested logic.

The one genuinely CH-native part is the **18 Phase-1 SQL invariants** (`sql/*.sql`):
they must be re-expressed for CH idioms — `FINAL` / `argMax(_, version)` over
`ReplacingMergeTree`, no foreign keys (orphan checks via anti-joins), `intDiv`
partitions, `Int64` cityhash surrogates, `Decimal128(7)` amounts. This is a
rewrite whether you call it port or not, because the SQL genuinely differs.

CH schema authority: `crates/db-clickhouse/schema/init.sql` +
`docs/architecture/database-schema/clickhouse-pilot.md`.

## Implementation Plan

### Step 1: DB-read adapter → ClickHouse

Swap the ~32 `sqlx` call sites in the 3 binaries for a `clickhouse::Client`
(local + mTLS prod), keeping the Horizon / archive / XDR diff logic intact.
Port the CLI surface (`clap`).

### Step 2: Phase-1 invariants re-expressed for CH

Rewrite the 18 per-table invariant SELECTs as ClickHouse SQL (`FINAL`/`argMax`
version collapse, no-FK anti-join orphan checks, partition-pruned scans). Keep
the `(violations, sample)` contract and the 0/1/2 exit-code convention.

### Step 3: Drop sqlx + close the 0244 carve-out

Remove `sqlx` from `crates/audit-harness/Cargo.toml`, then drop the `sqlx`
workspace dependency (0244 item 7) once audit-harness is the last user gone.

### Step 4: Wire into the correctness pipeline

Decide run cadence (manual `reports/` runs vs a scheduled job) and funnel
findings into the bug-task pipeline, same as before.

## Acceptance Criteria

- [ ] The 3 diff binaries read ClickHouse via `clickhouse::Client`, no `sqlx`
- [ ] Phase-1 invariants run against the live CH schema, `0 violations` on a clean store
- [ ] A saved report is produced under `reports/` and green on current prod data
- [ ] `sqlx` removed from `crates/audit-harness/Cargo.toml`; **workspace `sqlx` dep dropped** (closes 0244 item 7)
- [ ] **Docs updated** — `N/A` unless the port changes the shape of the system
- [ ] **API types regenerated** — `N/A` (no `crates/api/**` change)

## Notes

- Re-scoped from task **0244** (remove Postgres/sqlx entirely): audit-harness is
  kept + ported, not deleted. The crate is live in-tree; the diff-binary logic
  and the 18 `sql/*.sql` invariants are the starting point.
- Original crate: `crates/audit-harness` (task 0175 Phase 1+).
- Optional: consider whether the manual-audit skill `compare-with-stellar-api`
  (also PG-era) should be refreshed alongside.
