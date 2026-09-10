---
id: '0547'
title: 'FEATURE: order the assets list by holder count, not by the storage key'
type: FEATURE
status: completed
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
  - date: '2026-09-08'
    status: completed
    who: karolkow
    note: >
      Deployed and verified on the live list. Five code lines changed in one
      driver query plus the cursor it implies; 281 API tests pass, 3 rewritten
      where they asserted the old order. All 7 acceptance criteria met.
      Cost measured on production before shipping (61 ms / 1.02 M rows per
      page) and the mutable-key property of the new cursor measured, decided
      and recorded rather than left implicit.
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

## Implementation (2026-09-08)

One driver query changed and the cursor with it; the two-phase shape, the
version dedup and the hydration are untouched.

- `build_list_seek_sql` gains `LEFT JOIN balance_aggregates` and orders on
  `coalesce(ba.holder_count, -1) DESC, a.id DESC`. The fold lives in one
  `HOLDER_RANK` const so the ORDER BY and the cursor comparator cannot drift —
  a test asserts it appears exactly twice.
- `AssetKeyCursor` becomes `(holder_rank, id)`. An old cursor does not
  deserialize into that shape, so it fails at decode: **verified, HTTP 400
  `invalid_cursor`**, not a silent mis-page.
- The consecutive dedup still holds: both physical versions of an asset carry
  the same aggregate row, so they stay adjacent under the new walk exactly as
  they were under the primary-key walk.
- Two fields died with the old keyset and were removed rather than left
  dangling: `AssetKeyChRow.holder_rank` (the projection is unnecessary — the
  ORDER BY names the expression, and a struct field nothing reads is a decode
  liability against `DESCRIBE`) and `AssetRow.issuer_id` (read only to mint the
  old cursor).

**Verified against production through the local API**, not from SQL alone:

| Check                              | Result                                                                           |
| ---------------------------------- | -------------------------------------------------------------------------------- |
| page 1                             | XLM 9 949 966, TXT 804 336, USDC 684 175, AQUA 129 384, DRA, MFN, GTN, SHX       |
| native appears once                | yes — the dedup holds under the new order                                        |
| page 2 continues, no repeat or gap | 53 253 → 27 420, strictly below page 1's 54 816                                  |
| cursor from the OLD order          | **400 `invalid_cursor`**                                                         |
| `filter[code]=usdc`                | 684 175 USDC, then yUSDC, ExtraUSDC, USDC — ordered by holders within the filter |
| `filter[type]=soroban`             | MERU 21 379, eurSAFO, BLT, EUTBL                                                 |
| assets with no aggregate row       | 4 222 of them, rank `-1`, present and last                                       |

**Cost re-measured on the statement the API actually issues: 61 ms / 1.02 M
rows / 21 MiB.** Ordering on a joined column reads all of `assets`; against the
2 bn-row hourly quota that is ~1 960 page views an hour. The cheaper shape, if
that ever binds, is a driver over `balance_aggregates` itself (11 ms) — not a
return to the alphabet.

## The cursor now walks a MUTABLE key — known, measured, accepted

Raised by the task owner on review, and it is a real property the old order did
not have: the identity 4-tuple is immutable, so a keyset over it could only be
disturbed by inserts and deletes. `holder_count` changes on its own.

`balance_aggregates` is a refreshable MV on `REFRESH EVERY 2 MINUTE`, full
recompute + atomic `EXCHANGE`. So a single query always sees one consistent
snapshot; two pages fetched more than two minutes apart may straddle two.

Across a boundary, a count that moves past the cursor's value means the row
moves with it: upward → onto a page already shown, so it is **missed**;
downward → below the cursor, so it is **seen twice**. Bounded to rows whose
count crossed that exact value, never a whole page.

**Measured on production, 2026-09-08** — the top 60 rows (three pages), sampled
twice ~6 minutes apart:

|                                   |             |
| --------------------------------- | ----------- |
| assets whose holder count changed | **5 of 60** |
| positions that reordered          | **0**       |

Counts move constantly; the ORDER barely does, because near the top the gaps
are enormous (XLM 9 949 976, TXT 804 336, USDC 684 185 — a ±10 drift reorders
nothing) and only one asset sits within ±15 of the page-1 boundary.

The tail is the opposite shape and is where the effect is real: **335 735 of
452 599 assets hold 1–3 holders**, so there a ±1 change jumps a row past
thousands of ties. Nobody paginates to page 16 000, and the `id` tiebreak keeps
equal counts stably ordered, so the exposure is theoretical rather than
observed.

**This list is not the first.** `/accounts` already walks
`ORDER BY last_seen_ledger, id` with the same shape of cursor — and that key
moves on every transaction an account makes, far more volatile than a holder
count refreshed every two minutes. The property is not new to the system.

**Decision (task owner, 2026-09-08): accept.** The options weighed:

|     | Option                                       | Verdict                                                                                                                                                                   |
| --- | -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A   | accept                                       | **taken** — measured zero reorderings where readers look                                                                                                                  |
| B   | bucket the key (`intDiv(holder_count, 100)`) | rejected — kills churn nobody sees by making the order inside a bucket arbitrary, which readers do see                                                                    |
| C   | pin an aggregate version in the cursor       | the only strictly correct fix, and the expensive one: the MV keeps no old versions, so it means retaining, expiring and evicting snapshots — server state on a public API |
| D   | slow the refresh to ~15 min                  | rejected — buys list stability with staleness on the asset detail page, where the count is the content                                                                    |
| E   | `OFFSET`                                     | strictly worse: same defect, slower with depth, against ADR 0008                                                                                                          |

**What would reopen it:** a report of rows going missing while paging, not the
theory. The answer then is C.

## Deployed and verified on production (2026-09-08)

Read off the live list, 20 rows, strictly descending, native once:

```
XLM 9 950 045 · TXT 804 336 · USDC 684 236 · AQUA 129 397 · DRA 107 800 …
```

The deploy also produced the decision record's own evidence, unplanned. Against
the measurement taken an hour earlier: XLM +69, USDC +51, SHX +7, **DOGET −1**
— five counters moved, one downward, and **the order did not change**. Exactly
the property the "accept" decision rests on, observed rather than argued.

## Acceptance criteria

- [x] Default order is holder count, highest first; verified against the live
      API, not against the SQL
- [x] Native XLM appears ONCE — the dedup is asserted by a test, since the
      first measurement got six copies
- [x] Paging is stable across a boundary: no row seen twice, none skipped
- [x] A stale cursor from the old order is rejected, not silently mis-paged
- [x] Assets with no `balance_aggregates` row still appear, ordered last
- [x] Read cost re-measured on production before exposure (0243/0386 were both
      read-shape outages)
- [x] Docs updated — `docs/architecture/database-schema/endpoint-queries-clickhouse/**`
      and `frontend/**` per ADR 0032
