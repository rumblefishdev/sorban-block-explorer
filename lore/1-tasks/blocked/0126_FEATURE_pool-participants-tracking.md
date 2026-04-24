---
id: '0126'
title: 'LP: pool participants and share tracking'
type: FEATURE
status: blocked
related_adr: []
related_tasks: ['0052', '0077', '0136', '0162']
blocked_by: ['0162']
tags: [priority-low, effort-medium, layer-indexer, layer-db, audit-gap]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
  - crates/xdr-parser/src/state.rs
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — tech design specifies pool participants table on LP detail page but no schema exists.'
  - date: '2026-04-24'
    status: blocked
    who: stkrolikiewicz
    by: ['0162']
    note: >
      Real blocker retargeted. 0136 was listed as blocker but is
      superseded; the actual prerequisite is parser work emitting
      pool_share trustlines as ExtractedLpPosition rows (today
      skipped at xdr-parser/src/state.rs:231-234). Spawned 0162 to
      cover that parser gap. Once 0162 lands, 0126 owns persist +
      API for per-provider share tracking.
  - date: '2026-04-24'
    status: blocked
    who: stkrolikiewicz
    note: >
      Scope reconciled with existing schema. `lp_positions` table
      from migration 0006 §16 already carries the exact shape tech
      design requires (`pool_id`, `account_id`, `shares`,
      `first_deposit_ledger`, `last_updated_ledger`). Earlier README
      draft proposed a new `liquidity_pool_participants` table —
      that was redundant. No DDL change needed; task is now
      persist + API only.
---

# LP: pool participants and share tracking

## Summary

The technical design specifies a "Pool participants" section on the LP
detail page listing liquidity providers and their share
(`docs/architecture/technical-design-general-overview.md:224`,
`frontend/frontend-overview.md:492`). The `lp_positions` table already
exists from migration 0006 §16 with the exact shape required
(`pool_id`, `account_id`, `shares`, `first_deposit_ledger`,
`last_updated_ledger`, PK `(pool_id, account_id)`, partial index
`idx_lpp_shares (pool_id, shares DESC) WHERE shares > 0`). Today it is
always empty because the parser drops `pool_share` trustlines
(`xdr-parser/src/state.rs:231-234`). Task 0162 (prereq) closes the
parser gap; this task owns persist behaviour on top of that data and
the API surface.

## Context

Schema is already in place — no new table. Tech design asks only for
"providers and their share"; no withdrawal history, tx hash, or
removed_at_ledger is requested, and "Recent transactions" on the LP
page is a separate section sourced from `operations` /
`soroban_events`. Everything needed maps onto `lp_positions` as-is.

## Implementation

### 1. Persist behaviour (depends on 0162)

Once 0162 emits `ExtractedLpPosition` rows, wire an
`upsert_lp_positions` step into `persist/write.rs` (staging already
builds `LpPositionRow` at `staging.rs:706-716`). Semantics:

- Insert on first deposit; `first_deposit_ledger` set once, preserved
  on replay via `COALESCE`.
- Update `shares` and `last_updated_ledger` watermark-guarded
  (`GREATEST`) so older replays cannot roll state back.
- Withdrawal to zero: rely on the partial UNIQUE index predicate
  (`WHERE shares > 0`) + `DELETE WHERE shares = 0` pattern, or
  allow zero rows to persist and let the API's `shares > 0` filter
  hide them. Pick one at implementation; document in task notes.

### 2. API endpoint

`GET /liquidity-pools/:id/participants` — paginated list of
`(account StrKey, shares, first_deposit_ledger, last_updated_ledger)`
ordered by `shares DESC`. Uses `idx_lpp_shares` directly. Cursor
pagination matches the rest of the API (tasks 0043 / 0052).

### 3. Tests

- **Integration**: synthetic ledger with two providers for one pool
  → `lp_positions` has two rows with correct shares.
- **Integration**: replay / watermark — older `last_updated_ledger`
  cannot overwrite newer.
- **API**: endpoint returns participants sorted by `shares DESC`,
  filters zero-share rows per chosen withdrawal semantic.

## Acceptance Criteria

- [ ] `upsert_lp_positions` wired into persist path (watermark-guarded,
      `first_deposit_ledger` preserved on update).
- [ ] Withdrawal / zero-share handling documented and tested.
- [ ] `GET /liquidity-pools/:id/participants` returns per-provider
      shares, sorted by share size, cursor paginated.
- [ ] `lp_positions` populated end-to-end from `pool_share` trustlines
      after 0162 lands.
- [ ] Does NOT introduce `liquidity_pool_participants` — earlier draft
      suggested a new table; reconciled to existing `lp_positions`.
- [ ] Zero DDL change required — confirmed by tech design scope match
      on existing columns.
