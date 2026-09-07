---
id: '0530'
title: 'REFACTOR: `legs` as the only leg source — backfill classic, migrate the readers, drop the pair columns'
type: REFACTOR
status: backlog
related_adr: ['0058']
related_tasks: ['0374']
tags:
  [
    'clickhouse',
    'api',
    'frontend',
    'schema',
    'phase-future',
    'effort-large',
    'priority-medium',
  ]
links: []
history:
  - date: 2026-09-01
    status: backlog
    who: karolkow
    note: >
      Spawned from 0374. The end state and its four steps were decided and
      recorded inside 0374 while the soroban work was in flight, but never
      given a task of their own — so the "do not add new readers" rule had no
      owner. Filed now because the review of PR #438 had to add a
      `pool_kind = 0` guard that exists ONLY because the pair columns survive,
      and that guard is meant to die here.
---

# REFACTOR: `legs` as the only leg source, and the pair columns dropped

## Summary

`liquidity_pools` carries two representations of the same fact. The six
**LEGACY pair columns** (`asset_a_type/code/issuer_id`, `asset_b_*`) describe a
two-leg pool, and `legs Array(Int64)` describes any pool. Classic rows fill the
pair columns; soroban rows fill `legs`. The end state — decided in 0374, not
open — is **`legs` as the only leg source in both worlds, with the six columns
dropped**.

Two facts make the pair shape wrong rather than merely redundant: 3- and 4-leg
stable pools exist on mainnet, and a pair cannot express them; and a soroban row
must put SOMETHING in the pair columns, so it writes defaults that read as
meaningful values downstream.

## Context — why this is now worth its own task

The schema says `-- LEGACY pair shape` on all six columns and carries the rule
**"do not add new readers"**, but that rule had no task behind it, so it relied
on whoever happened to read the comment.

The review of PR #438 (2026-09-01) showed what the survival costs. A soroban
registry row writes `asset_a_type = 0`, and `0` in that column means _native
XLM_ to every classic reader — so the shared asset-code predicate rendered
**every soroban pool as `XLM/XLM`**. Measured on production, live:

|                  | classic (52,677)        | soroban (497)         |
| ---------------- | ----------------------- | --------------------- |
| leg A type 0     | 11,735 — genuine native | **497 — placeholder** |
| leg B type 0     | **0**                   | **497 — placeholder** |
| both legs type 0 | **0**                   | **497**               |

A classic pool never has both legs at type 0 (a pool cannot be XLM/XLM, and
CAP-38 orders the pair so native sorts first). A soroban pool always does. So
an `XLM` filter returned 15,005 real classic pools plus **497 false ones**, and
an `XLM/XLM` filter returned 754 plus the same 497.

That was fixed by gating the predicate to `pool_kind = 0` — correct as an
interim, since the predicate reads only the legacy columns and those are
classic-only, but it is a guard that exists solely because this migration has
not happened. **It should be deleted as part of step 3, not preserved.**

## Implementation — the four steps recorded in 0374

Each step has its own verifier; they are ordered and cannot be reshuffled.

1. **Settle the surrogate question.** Determine whether our `hash64` surrogate
   equals ClickHouse's `cityHash64`. One sample comparison decides whether
   step 2 is a SQL mutation or a Rust job. (They are known NOT to be
   bit-equivalent in general — `cityhash-rs::cityhash_102_128` lower 64 bits
   versus the CH builtin — so assume the Rust job until measured otherwise.)
2. **Backfill `legs` for the ~52,620 classic rows**, versioned on each row's
   own `last_updated_ledger`. Kind 0 legs are ASSET surrogates
   (`pool_leg_asset_id`, the `lp_operation_amounts` join key), NOT the
   token-contract surrogates kind 1 uses — the id space is per-kind and
   `pool_kind` says which.
3. **Migrate the ~612 pair-shaped call sites** (API queries, classifier,
   frontend) to `legs`. The largest piece, and the one that lands
   incrementally behind a read-time coalesce bridge. `asset_codes_predicate`
   is rewritten here to match through `legs` + the asset dimensions, which
   makes it work uniformly for both kinds — and the `pool_kind = 0` guard
   inside it disappears with the columns it protects.
4. **Drop the columns.** `ALTER TABLE liquidity_pools DROP COLUMN
asset_a_type, …` (operator) together with the removal from `init.sql`, the
   row struct, and the column-order guard. These three can only move in
   lockstep — `column_order_liquidity_pools` enforces it — and `init.sql`
   cannot lose them earlier, because the driver validates inserts against the
   live table.

## Not in scope

- **`pool_kind` itself stays.** It is a real user-facing distinction: the
  classic/soroban list filter, and the id rendering (`L…` SEP-23 for classic,
  `C…` for soroban — the same 32 bytes rendered the wrong way produce a
  well-formed WRONG key). Only the guard inside the asset-code predicate goes.
- **The reserve-model unification** (`liquidity_pool_snapshots` joining into
  `pool_state_changes`, ADR 0058 §3) is a separate merge about reserves, not
  about leg identity. It can happen in either order relative to this task.

## Acceptance Criteria

- [ ] `legs` populated for every classic row, spot-verified against the pair
      columns before they are dropped
- [ ] No production reader references `asset_a_*` / `asset_b_*`
- [ ] `asset_codes_predicate` matches through `legs`, with no `pool_kind`
      guard, and returns the same classic results as today plus soroban pools
      matched by their real leg codes
- [ ] The `XLM` and `XLM/XLM` filters return no false positives for either
      kind (the 497 measured in #438 stay gone, and none appear for classic)
- [ ] Columns dropped from the live table, `init.sql`, the row struct and the
      column-order guard in one change
- [ ] **Docs updated** — `docs/architecture/database-schema/**` describes
      `legs` as the only leg source
