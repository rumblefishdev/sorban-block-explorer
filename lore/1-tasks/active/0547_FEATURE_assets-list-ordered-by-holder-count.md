---
id: '0547'
title: 'FEATURE: order the assets list by holder count, not by the storage key'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0364', '0450', '0331']
tags: [frontend, api, assets, clickhouse, priority-medium, effort-small]
links: []
history:
  - date: '2026-09-08'
    status: active
    who: karolkow
    note: >
      Raised on sight of the live list: the browse order is alphabetical inside
      each type and reads as noise. Not a regression — it has been the storage
      key since 0364 made the read two-phase; nobody chose it as a browse
      order, it fell out of choosing a cheap keyset.
---

# FEATURE: order the assets list by holder count

## Summary

`/assets` walks `(asset_type, asset_code, issuer_id, contract_id)` — the
`assets` primary key. That makes paging a key-prefix walk, which is why it was
chosen (task 0364), but as a BROWSE order it is alphabetical-within-type and
tells a reader nothing. Order by **active holder count** instead, so the assets
people actually hold come first.

## Context

No column in the assets table is sortable, so this order is the only one a
reader ever sees; the `order` parameter merely reverses the same alphabet.

`balance_aggregates` already holds `holder_count` per `assets.id` (task 0331) —
448 537 rows, one per asset, and the list already renders the column. So the
data exists; only the read order and its cursor change.

## Measured before deciding (production, 2026-09-08)

| Shape                                               | Time      | Rows read |
| --------------------------------------------------- | --------- | --------- |
| driver straight off `balance_aggregates`, no filter | **11 ms** | 448 537   |
| with a filter — must join `assets`                  | 174 ms    | 1 017 342 |

The unfiltered shape is CHEAPER than the account page's existing asset-identity
read (268 k rows), so this is not a cost trade at all in the common case.

**`assets` must be deduplicated on read.** Joining it without `LIMIT 1 BY id`
returns native XLM six times — the table holds 984 k physical rows for ~452 k
assets (unmerged `ReplacingMergeTree` parts, the standing rule in this repo).
The first measurement hit exactly that.

Sanity check of the resulting order: XLM 9 949 943, TXT 804 336, USDC 684 170,
AQUA 129 389, DRA 107 800, MFN 74 502.

## Scope

1. Order the list by `holder_count DESC` with the asset surrogate as the
   tiebreak, so the order is total and a page boundary is stable.
2. Cursor becomes `(holder_count, id)` instead of the identity 4-tuple. A
   cursor minted under the old order must fail clean, not mis-paginate
   (ADR 0008).
3. Assets with no aggregate row sort last — absence is not zero holders.
4. Filters (type / SAC / code) keep working; that path pays the join.

## Not in scope

- Making the column clickable / a second sort mode (option B, not taken —
  a browse list wants one good default, and the second mode doubles the cursor
  work).
- Any change to what `holder_count` MEANS; it comes from `balance_aggregates`
  as it is.

## Acceptance criteria

- [ ] Default order is holder count, highest first; verified against the live
      API, not against the SQL
- [ ] Native XLM appears ONCE — the dedup is asserted by a test, since the
      first measurement got six copies
- [ ] Paging is stable across a boundary: no row seen twice, none skipped
- [ ] A stale cursor from the old order is rejected, not silently mis-paged
- [ ] Assets with no `balance_aggregates` row still appear, ordered last
- [ ] Read cost re-measured on production before exposure (0243/0386 were both
      read-shape outages)
- [ ] Docs updated — `docs/architecture/database-schema/endpoint-queries-clickhouse/**`
      and `frontend/**` per ADR 0032
