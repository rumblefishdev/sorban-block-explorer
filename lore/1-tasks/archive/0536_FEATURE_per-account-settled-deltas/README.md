---
id: '0536'
title: 'FEATURE: per-account settled deltas — direction (received / sent) on the transaction lists'
type: FEATURE
status: superseded
related_adr: []
related_tasks: ['0393', '0411', '0538', '0419']
tags:
  [
    'clickhouse',
    'indexer',
    'api',
    'frontend',
    'phase-future',
    'effort-large',
    'priority-medium',
  ]
links:
  - crates/db-clickhouse/schema/init.sql
history:
  - date: 2026-09-03
    status: backlog
    who: karolkow
    note: >
      Spawned from 0411. The shipped "Net settled" column is always positive by
      construction — it reports value that changed hands per (transaction,
      asset), not what an account gained or lost, so on an account page it does
      not answer "did I receive or send this". Product decision (karolkow,
      2026-09-03) is the lossless option: store the signed delta per
      (transaction, asset, account). Cost MEASURED across three epochs before
      filing — 19.3 bn rows, 153 GB on the natural key, 249 GB with the
      surrogate id. Hard-ordered after [[0538]]: without the natural key the
      same table costs 96 GB more.
  - date: 2026-09-06
    status: superseded
    who: karolkow
    by: ['0540']
    note: >
      Superseded by 0540. Edges (one row per transfer) contain the node grain this task proposed at 0.28× the rows, and every measurement here — the per-column method, the `lp_operation_amounts` precedent, the free-space warning — was carried into 0540's research note and tickets. The `values[]` API question moved with it.
---

# FEATURE: per-account settled deltas

## Summary

Store the **signed delta per (transaction, asset, account)** so the transaction
lists can say what an account received or sent, not merely how much value moved.
This is the lossless option of the six considered — everything cheaper drops
information the account page needs.

## Context

Task 0393 chose `max(Σ+, Σ−)` per (transaction, asset) and 0411 shipped it to
every transaction list. That metric is correct for a **list without an account
in context** (the global list, a ledger) — it counts a routed payment once
rather than at every hop, per the flow decomposition theorem. It is the wrong
metric for an **account page**, where the reader's question is directional.

The reported case makes it concrete: the bridge-in on 2026-09-01 and the pool
deposit the next day are opposite in direction and identical on screen.

## Why not the cheaper options

Six options were compared. The two that looked attractive both fail:

**Two columns, Σ+ and Σ− per (transaction, asset) — ~300 MB.** Rejected on
measurement, not on principle. Decoding 24 transactions from raw XDR:

|                        | count           |
| ---------------------- | --------------- |
| `Σ+` **equals** `Σ−`   | 42 / 64 (65.6%) |
| `Σ+` differs from `Σ−` | 22 / 64 (34.4%) |

For an ordinary transfer the asset has to land somewhere, so the two sums are
the same number and the second column carries nothing. Where they differ it is
**mint and burn** (KALE farming shows `+6,984,583 / −0`), so the column would
answer "was this issuance or destruction", not "did this account receive". It
does not solve the stated problem.

**Signed against the transaction's source account — ~300 MB.** Works only when
the reader is the sender, silently wrong for the recipient and for
fee-bump/sponsored transactions. A half-measure that looks like a full one.

Per-**operation** grain (rather than per-transaction) was also rejected: ~2×
the rows for detail the list never renders, and the transaction detail page
already has the full XDR.

## Measured cost

Method and per-epoch figures: [notes/R-cost-measurement.md](notes/R-cost-measurement.md).

|                                       | value       |
| ------------------------------------- | ----------- |
| Rows                                  | **19.3 bn** |
| **With the natural key ([[0538]])**   | **153 GB**  |
| With the surrogate `transaction_id`   | 249 GB      |
| Penalty for skipping 0538             | **+96 GB**  |
| Share of today's free space (459 GiB) | ~31%        |

Sensitivity to the one remaining assumption (sides per Soroban transfer):
108 GB at 1.0, 173 GB at 2.0. Everything else is measured.

> **Order matters.** [[0538]] decides whether identity columns use
> `(ledger_sequence, application_order)` instead of the incompressible
> surrogate. Building this table first and migrating later means paying the
> 96 GB, then rewriting 19.3 bn rows to reclaim it.

## The shape already exists

`lp_operation_amounts` (task 0279) is the same table one dimension over: keyed
`(pool_id, ledger_sequence, transaction_id, application_order, asset_id)` with
`amount Int128` **signed from the pool's perspective**. 948 M rows, 11.6 GiB,
in production. Swap the pool for an account and the design is settled — the
per-column costs in the estimate above are taken from it, not modelled.

Proposed key: `(account_id, ledger_sequence, application_order, asset_id)`.
`account_id` leads because the account page is the reading pattern;
measured at **0.092 B/row** when it leads a key (`transaction_participants`).

## Implementation sketch

1. **Reducer.** The indexer already computes per-account deltas in memory and
   collapses them to one number before the insert (`ledger_deltas_net_settled`).
   Emit them instead of discarding them.
2. **Table.** New table, natural key, `delta Int128` signed. Store only
   non-zero deltas — a pass-through account nets to zero and needs no row
   (measured: 91.4% of touched triples actually move).
3. **Read path.** Account transactions endpoint joins on `account_id` —
   a primary-key seek, unlike today's `asset_id`-leading scan. Benchmark before
   exposing: 0243/0386 were both read-shape outages.
4. **Frontend.** Direction on the account page; the global list keeps the
   net-settled figure, which stays correct there.
5. **Backfill.** Full historical re-run, same machinery as [[0419]].

## Open questions

- **The reducer loses sides on Soroban pools.** When an asset enters a Soroban
  AMM pool the outflow from the account is visible but the pool's own balance
  (a contract-storage entry) may not be — seen while measuring, and it is why
  ~34% of sampled asset-transactions showed an unbalanced `Σ+`/`Σ−`. Some of
  that is genuine mint/burn, some is blind spots. Resolve before trusting a
  backfill; overlaps [[0374]].
- **Does the `values` API field change shape, or does a new one appear?**
  0411's contract serialises `values[]` without direction; adding a sign there
  is a breaking change for any consumer that assumes positive.
- **Free space.** 153 GB against 459 GiB free, with `/backups/` on the same
  volume. Sequence against the 0419 backfill so both do not peak together.

## Acceptance Criteria

- [ ] [[0538]] resolved first — identity column decided before any row is written
- [ ] Reducer emits per-account signed deltas; non-zero only
- [ ] Table created with the natural key; no surrogate `transaction_id`
- [ ] Soroban pool sides resolved or explicitly scoped out with a measured
      coverage figure — never a silently partial number
- [ ] Read path benchmarked on the account endpoint before it is exposed
- [ ] Account page shows direction; global list unchanged
- [ ] Historical backfill completed and cross-validated against raw XDR on a
      sample spanning all three epochs
