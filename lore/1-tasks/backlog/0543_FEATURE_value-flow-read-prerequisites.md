---
id: '0543'
title: 'FEATURE: value-flow read prerequisites — what the account page needs before rollout steps 8–10'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0540', '0541', '0453', '0243', '0386']
tags:
  [
    'clickhouse',
    'api',
    'frontend',
    'phase-future',
    'effort-medium',
    'priority-high',
  ]
links:
  - crates/db-clickhouse/schema/init.sql
  - crates/api/src/accounts/queries.rs
history:
  - date: 2026-09-07
    status: backlog
    who: karolkow
    note: >
      Spawned from 0540's deep review. The write path of the value-flow tables
      is complete; these are the things the read path needs that the write
      path does not provide, collected so that rollout steps 8–10 do not
      discover them one by one.
---

# Value-flow read prerequisites

## Summary

Everything the account page's `Balance change` column needs from the data
layer beyond the `asset_transfers` rows themselves: a way to name `L…` and
`B…` endpoints, a rule for contract-authored rows, the join shape and its
benchmark, and the staging-level oracle that 0540 left at the parser seam.
Ordered so that anything that must be decided **before** the backfill (a
column on a 5.47 bn-row table) is decided first.

## Context

0540 ships one row per token movement, keyed by the official event identity.
Its endpoints are `hash64(strkey)` surrogates with a `*_kind` letter; `G`
resolves through `accounts`, `C` through `soroban_contracts`, and **`L`
(classic pools, 12.4% of `to` endpoints on production) and `B` (claimable
balances, 3.5%) resolve through nothing** — `liquidity_pools` is keyed by the
raw 32-byte pool id and claimable balances have no table. A bespoke token's
`from`/`to` are whatever the contract wrote. The README's read benchmark
covered the `asset_transfers` leg only; the page reaches it through
`transaction_participants → transactions`.

## Implementation Plan

### Step 0 (before rollout step 2): decide `transaction_id` on `asset_transfers`

`event_index` is declared to join `soroban_events`, which needs
`transaction_id`; the row has `application_order` instead, so the join hops
through `transactions`. Precedent both ways: `lp_operation_amounts` carries
both. Decide before the table exists on production — adding a column to
5.47 bn rows later is a rewrite.

### Step 1: resolving side table for `L…` and `B…`

`(address_id, strkey)` for every pool and claimable-balance address that
appears as an endpoint. Buildable inside ClickHouse from
`soroban_events.topics_xdr` — no S3 pass.

### Step 2: contract-authored rows are marked

A row whose `asset_id` resolves to `asset_type = 3` is written by the
contract, not by stellar-core. The account page marks it and never renders
it by `symbol` alone (production: `symbol = 'USDC'` matches 4 contracts).

### Step 3: end-to-end read benchmark

`transaction_participants → transactions → asset_transfers` on
production-shaped parts, `FINAL` vs `GROUP BY` dedup, rows read against the
2 bn-row/hour quota (0243 and 0386 were this read shape). Consider
`index_granularity` on `transaction_participants`, which has the same
point-read shape and sits at the default.

### Step 4: staging-level oracle

A second oracle pass through `build_value_flow_rows` on real ledgers
(surrogates, `*_kind`, muxed matching, `narrow()`), following
`pool_state_stage_real_e2e.rs`; split the parser oracle's `no_witness` into
`by_layout` vs `event_only_bespoke`.

### Step 5: retire the dead `net_settled` chain

`ExtractedTransaction.ledger_deltas`, `stage::ledger_deltas_net_settled`,
`net_settled.rs`, `net_settled_real_e2e.rs` (~587 LOC, zero production
readers) — its own decision, after T03's reader is confirmed as the oracle
witness only.

## Acceptance Criteria

- [ ] `transaction_id` on `asset_transfers` decided and recorded before the
      table is created on production
- [ ] Every `L…` / `B…` endpoint on the account page renders as its StrKey
- [ ] Contract-authored rows are visibly marked; no rendering by symbol alone
- [ ] Read benchmark on production-shaped parts recorded, with the dedup
      choice
- [ ] Staging oracle passes on the 0540 ledger list
- [ ] **Docs updated** — `docs/architecture/database-schema/**`,
      `frontend/**` per ADR 0032
- [ ] **API types regenerated** — when the account endpoint changes

## Notes

Tracked here but owned by 0540's rollout (before step 6): MEMO_TEXT that is
not valid UTF-8 stored as bytes (`memo_type = "text_hex"`); `to_muxed_id`
inherited only by the transfer whose asset matches the operation's.
