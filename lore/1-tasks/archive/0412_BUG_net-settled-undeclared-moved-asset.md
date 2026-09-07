---
id: '0412'
title: 'BUG: net-settled drops value for an asset that moved but the op did not declare'
type: BUG
status: superseded
related_adr: []
related_tasks: ['0393']
tags:
  [
    'clickhouse',
    'indexer',
    'xdr-parser',
    'phase-future',
    'effort-small',
    'priority-medium',
  ]
links: []
history:
  - date: 2026-07-20
    status: backlog
    who: karolkow
    note: 'Spawned from 0393 deep review (finding #8). Low: exotic ops only.'
  - date: 2026-07-20
    status: backlog
    who: karolkow
    note: 'Widened: reach is broader than exotic classic ops — value is ledger-sourced but visibility is still op/event-GATED, so a Soroban invocation that moves value via ContractData with no parseable event and no op declaration also drops. Bumped low → medium.'
  - date: 2026-09-06
    status: superseded
    who: karolkow
    by: ['0540']
    note: >
      Superseded by 0540. The `net_settled` column was removed on 2026-09-04; the live half of this bug — the ledger reader blind to classic pools and claimable balances — is fixed there (`LiquidityPoolEntry` and `ClaimableBalanceEntry` as holders, reconciled by the events-vs-ledger oracle with 0 contradictions on 33 ledgers).
---

# BUG: net-settled drops value for an asset that moved but the op did not declare

## Summary

The net-settled value is attached only to the `operation_asset_appearances`
presence rows an **operation body declared** (`op.asset_appearances`). The value,
however, is keyed independently by whatever `ledger_balance_deltas` actually
moved. If an asset has a real net delta but appears in **no** op's declared
asset set, no presence row is emitted and the computed value is silently dropped.

## Context

- Write site: `crates/db-clickhouse/src/persist/stage.rs` (~1041-1067) — the row
  key set is `op.asset_appearances`; `net_settled` is looked up from
  `amount_by_tx_asset` (keyed by the deltas).
- The two key sets (declared presence vs actual balance deltas) are computed
  independently, so a moved-but-undeclared asset falls through. **Value is now
  ledger-sourced, but its VISIBILITY is still gated** on an op-declared OR
  event-declared presence row — there is no writer that iterates the
  ledger-derived value set (`amount_by_tx_asset`) directly.
- **Reach (broader than first scoped):**
  - **Classic exotic ops** — an op that shifts a reserve without declaring the
    asset, e.g. `RevokeSponsorship` moving a base-reserve, produces a real native
    `classic_delta` but no native `asset_appearance`.
  - **Soroban no-event case (headline)** — a Soroban invocation that moves value
    via a `ContractData` balance change whose token emits no parseable SEP-41/CAP-67
    event, and which no op body declares, produces a real ledger delta with no
    presence row → its value drops. This is the same "value from ledger, visibility
    from logs" seam, not an exotic edge.
  - Fails safe: it under-emits (blank), never a wrong figure.

## Implementation

- Reconcile the value key set with the presence key set: for any `(tx, asset)` in
  `amount_by_tx_asset` with no declared presence row, decide whether to emit a
  presence row so the value surfaces (note: this changes presence-row semantics —
  a row appears that op-declaration would not have produced; weigh against the
  0383/0359 presence model before doing it).
- Alternatively, document the gap as accepted and leave exotic-op value as `NULL`.

## Acceptance Criteria

- [ ] A `RevokeSponsorship` (or similar reserve-moving op) surfaces its native
      net-settled value, OR the gap is explicitly documented as accepted with a
      rationale tied to the presence-row model.
- [ ] No regression to the op-declared presence rows / their dedup against events.
