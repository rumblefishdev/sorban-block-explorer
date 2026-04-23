---
id: '0159'
title: 'REFACTOR: drop `account_balance_history` — unused, defer chart feature design'
type: REFACTOR
status: active
related_adr: ['0035']
related_tasks: ['0157', '0158']
tags: [schema, write-path, size-reduction, drop-unused]
links:
  - lore/2-adrs/0035_drop-account-balance-history.md
  - lore/1-tasks/archive/0158_REFACTOR_soroban-invocations-appearances.md
history:
  - date: '2026-04-23'
    status: backlog
    who: fmazur
    note: 'Task created — decision made with senior to drop unused balance-history table.'
  - date: '2026-04-23'
    status: active
    who: fmazur
    note: >
      Activated as current task immediately after 0158 closed. ADR 0035
      already drafted as `proposed` alongside the task; will flip to
      `accepted` after implementation + bench comparison.
---

# REFACTOR: drop `account_balance_history` — unused, defer chart feature design

## Summary

Drop the `account_balance_history` table and all its write-path code paths.
The table has zero production consumers (no endpoint in ADR 0021 reads it;
only projected as support for a "future balance-over-time chart" that is
not yet in spec). Keeping its population costs ~10-20 ms/ledger on the
write path and an estimated 90 GB–1.1 TB of disk at full 11M-ledger scale.

`account_balances_current` stays — it's the hot-read path for E6 (account
balances) and E8 (token supply + holder_count aggregates). The denormalisation
between `current` and `history` was inherited from ADR 0012/0027; the decision
here is to collapse it to a single authoritative table (`current`) and
reintroduce a historical index if/when the chart feature lands, with a
design chosen against actual query patterns.

## Status: Backlog

**Current state:** Decision made (2026-04-23), picked up after task 0158
closes. Task 0157 and 0158 tie up the soroban_events/invocations appearance
refactor; this task cleans up the account-balances side which was the
second-biggest write-time contributor per the 0158 benchmark work.

## Context

Per audit during task 0158:

- `balances_ms` averaged ~38 ms per ledger out of ~200 ms total persist time
  (19%, second-largest stage after `operations_ms`).
- Breakdown of `balances_ms`: ~5 sub-queries per ledger — trustline DELETEs,
  current-native UPSERT, current-credit UPSERT, history-native INSERT,
  history-credit INSERT. The two history INSERTs (steps 14c-N and 14c-C)
  write ~724 rows/ledger average.
- Empirical scaling: ~72k rows per 100 ledgers; projections for 11M ledgers
  diverge between my linear extrapolation (~1.1 TB) and ADR 0020's 90 GB
  estimate. Either way: largest wasted write-only table in the project.
- ADR 0021 cross-reference: `account_balance_history` appears twice —
  line 87 (schema table row 18) and line 391 (noted as "supports a future
  balance over time chart"). **Zero endpoints read from it.**
- Empirical verification: `account_balances_current.last_updated_ledger`
  equals `MAX(account_balance_history.ledger_sequence)` across all sampled
  accounts; the two tables are a projection relation — current is the
  latest-per-(account,asset) projection of history.

The E6 balance section and E8 token-stats aggregates read from
`account_balances_current` directly. After this refactor nothing changes
from the API perspective — only the denormalised history disappears from
the write path.

## Scope

### In scope

1. **Migration in-place rewrite** — drop `CREATE TABLE account_balance_history`
   block (and its two partial unique indexes `uidx_abh_native` /
   `uidx_abh_credit`) from `0007_account_balances.sql`. Rewrite-in-place
   convention (no production DB).
2. **Domain cleanup** — remove `AccountBalanceHistory` struct from
   `crates/domain/src/balance.rs` + update doc comments.
3. **Staging simplification** — remove `balance_history_rows: Vec<BalanceRow>`
   field from `Staged`; remove the `clone_balance_row` loop and helper at
   staging.rs ~line 948-989.
4. **Write path cleanup** — remove `append_balance_history`,
   `append_balance_history_native`, `append_balance_history_credit` from
   write.rs. Remove the call site from `upsert_balances` (write.rs ~1631).
   Keep 14a (trustline DELETE) and 14b (current UPSERT).
5. **Test cleanup** — remove balance-history from `persist_integration`:
   `Counts.balance_history`, the CTE `bh`, partition bootstrap loop
   entry, DELETE FROM in clean_test_ledger.
6. **Partition management** — remove `"account_balance_history"` from
   `TIME_PARTITIONED_TABLES` in `crates/db-partition-mgmt/src/lib.rs`.
7. **Backfill-bench** — remove from default-partition list in
   `crates/backfill-bench/src/main.rs`.
8. **ADR 0021 updates** — remove row 18 from schema table; remove the
   "future chart" reference at line 391; update references to consolidate
   balance-info to `account_balances_current` only.
9. **ADR 0027 update** — mark §18 as superseded by ADR 0035.
10. **ADR 0035 draft** — new ADR documenting the decision + rationale +
    alternatives considered (appearance pattern, S3 snapshots, XDR replay),
    plus plan for feature-launch-time redesign.

### Out of scope

- **Balance-chart feature design** — explicit non-goal. When the feature
  enters backlog we re-decide whether to materialize snapshots, derive
  from XDR replay, use an appearance index, or something else, with load
  numbers from real expected query patterns.
- **Changes to `account_balances_current`** — stays as today, including
  watermark UPSERT and trustline DELETE logic.
- **Reconstruction of historical snapshots** — if a future feature needs
  it, the fallback is re-ingest of target ledger range (idempotent via
  ON CONFLICT) or targeted XDR replay per ADR 0029 pattern.

## Implementation Plan

Mirrors 0157/0158 pattern — schema + write-path + tests + ADRs together.

1. **Draft ADR 0035** — status `proposed`, accepted after implementation.
2. **Migration** — in-place rewrite of `0007_account_balances.sql`.
3. **Staging** — drop `balance_history_rows` from `Staged` + builder.
4. **Write** — drop history-append code, keep only 14a + 14b.
5. **Domain** — drop `AccountBalanceHistory` type.
6. **Partition-mgmt + backfill-bench** — remove table name.
7. **Integration test** — drop history counts + cleanup + CTE.
8. **ADR 0021 + ADR 0027** — cross-ADR updates.
9. **Full Rust gate** — `cargo build --workspace`, `cargo test --workspace --lib`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.
10. **Re-bench** — compare `balances_ms` before/after; target: eliminate
    the ~10-20 ms/ledger 14c component.

## Acceptance Criteria

- [ ] `account_balance_history` no longer exists in migrations, code, or
      domain types.
- [ ] `persist_integration` does not reference balance_history; all its
      existing counts assertions still pass.
- [ ] `balances_ms` stage drops measurably (target: −10 to −20 ms/ledger
      on the same 100-ledger sample used in task 0158).
- [ ] `cargo build --workspace`, `cargo test --workspace --lib`,
      `cargo clippy --workspace --all-targets -- -D warnings`, and
      `cargo fmt --all -- --check` all green.
- [ ] ADR 0021 no longer references `account_balance_history`.
- [ ] ADR 0027 §18 carries a superseded-by-0035 marker.
- [ ] ADR 0035 accepted after implementation with measured speedup noted.

## Notes

- **Trustline removal caveat is not an issue here** — because
  `account_balances_current` stays, the DELETE-on-removal path continues
  to keep live state correct. Trustline history is lost (was never queried)
  but that's the whole point of this refactor.
- **Reversibility:** trivial — re-add migration block, re-add write code,
  backfill via re-ingest of target range (idempotent upserts).
- **Storage baseline to measure:**
  - Current 100-ledger sample: ~10 MB total (5.4 MB heap + 4.9 MB indexes).
  - Post-drop: 0.
- **Write-time baseline to measure:**
  - Current: ~15-20 ms/ledger attributable to history writes (14c-N + 14c-C).
  - Post-drop target: ~0 ms (entire 14c stage removed).
- Task 0158 benchmark harness reusable verbatim for the before/after measure.
