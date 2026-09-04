---
id: '0540'
title: 'FEATURE: lossless value-flow index — per-transfer edges replacing the net-settled aggregate'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0393', '0411', '0412', '0413', '0419', '0536', '0538']
tags:
  [
    'clickhouse',
    'indexer',
    'api',
    'frontend',
    'phase-future',
    'effort-large',
    'priority-high',
  ]
links:
  - crates/xdr-parser/src/ledger_value.rs
  - crates/db-clickhouse/schema/init.sql
history:
  - date: 2026-09-04
    status: backlog
    who: karolkow
    note: >
      Filed after removing the `net_settled` column the same day. The
      per-(transaction, asset) aggregate carried no direction and no account, so
      an inbound and an outbound transfer rendered identically on an account
      page. Replacement is per-transfer edges. Scope set by the task owner:
      design closed AND phase 1 (edges from the events already in ClickHouse, no
      operation attribution) live in production; phase 2 (operation attribution
      + historical reconciliation, both needing S3) rides [[0419]]. Asset-led
      flow tracing and L5/Effects are explicitly out of scope. Planning is a
      wayfinder map at `.wayfinder/value-flow/` (local-only, gitignored).
---

# Lossless value-flow index

## Summary

Store one row per **transfer** — `from`, `to`, `asset`, `amount` — instead of one
aggregate per (transaction, asset), so the transaction lists can say what an
account received or sent, and so no information is lost between what the chain
recorded and what we index.

## Context

Task 0393 chose `max(Σ+, Σ−)` per (transaction, asset) and 0411 put it on every
transaction list. Two separate problems retired it:

1. **No direction, no account.** The figure is a property of the transaction, so
   an account page renders a received and a sent transfer identically.
2. **It was measurably wrong for 7.6% of value-moving transactions** — not
   because of the formula, but because the reader feeding it is blind to classic
   liquidity pools.

Measurements behind both, with method:
[notes/R-events-vs-ledger-reconciliation.md](notes/R-events-vs-ledger-reconciliation.md).

## What is settled

- **Edges, not nodes.** One row per transfer (~9.05 bn rows measured directly
  from stored events), not one per (transaction, asset, account) (19.3 bn, task
  0536). Edges reconstruct nodes; nodes do not reconstruct edges.
- **Phase 1 needs no S3.** Token events already in ClickHouse cover 100% of
  value-moving transactions across the whole ingested range, pre-CAP-67 included.
- **`event_index` is part of row identity.** Identical transfers repeat inside a
  single transaction, and — proven on production — inside a single **operation**.
  Nothing coarser than the event ordinal can tell them apart.
- **Row identity: `event_index`, permanently** (2026-09-04). Stellar's official
  identity (`op_index` + position within the operation) was proven **available** —
  1 274 transactions decoded across protocols 20, 22 and 27, 1 770 non-diagnostic
  token events, none lacking an operation index — and then deliberately not made
  the sort key: a portable identity earns its keep as a column, since reconciling
  against `getEvents` is a read-time join. It rides along as nullable columns the
  phase-2 S3 pass fills, so phase 2 never rewrites a row.
- **The ledger reader gets fixed** (option A, task owner, 2026-09-04):
  `LiquidityPoolEntry` and `ClaimableBalanceEntry` are added to
  `ledger_value.rs`, so the reconciliation witness is complete rather than
  firing on our own blind spot. Absorbs the live half of [[0412]] and [[0413]].

## What is still open

Nine decision tickets on the map at `.wayfinder/value-flow/` — identity key,
table shape, reconciliation rule, size, rollout, what the column renders, and the
disposition of [[0536]]. The map is local-only by convention; this task is the
committed record of the work it plans.

## Acceptance Criteria

- [ ] Edge table exists, keyed so that identical transfers in one operation stay
      distinct rows — verified against the measured cases, not by argument
- [ ] `event_index` proven stable across a re-ingest BEFORE the backfill runs —
      it is row identity now, and its determinism is so far only asserted
- [ ] `ledger_value.rs` reads `LiquidityPoolEntry` and `ClaimableBalanceEntry`;
      the 7.6% discrepancy is re-measured and accounted for
- [ ] Reconciliation flag written at ingest; a transaction that does not
      reconcile is never rendered as a complete number
- [ ] Phase-1 backfill covers the full ingested range, with coverage **proven**
      against an independent source, not inferred from a row count
- [ ] Read path benchmarked on the account endpoint before exposure
      (0243/0386 were both read-shape outages)
- [ ] Direction visible on the account page in production
- [ ] **Docs updated** — `docs/architecture/database-schema/**`,
      `indexing-pipeline/**`, `xdr-parsing/**`, `frontend/**` per ADR 0032
